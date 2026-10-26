use super::workflow::{workflow_job_blocks, workflow_step_blocks};
#[cfg(test)]
use std::fs;

fn source(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

fn has_permission(job: &str, permission: &str) -> bool {
    job.lines().any(|line| line.trim() == permission)
}

pub(super) fn metadata_recovery_auth_contract(
    workflow: &str,
    release: &str,
    recovery: &str,
    process: &str,
) -> std::result::Result<(), String> {
    let workflow = source(workflow);
    let release = source(release);
    let recovery = source(recovery);
    let process = source(process);
    let jobs = workflow_job_blocks(&workflow);
    let job = |id: &str| {
        jobs.iter()
            .find(|(job_id, _)| job_id == id)
            .map(|(_, body)| body.as_str())
            .ok_or_else(|| format!("metadata recovery auth contract is missing job {id}"))
    };
    let plan = job("plan")?;
    let apply = job("apply")?;
    let declares_write_scope = |body: &str| {
        body.lines().any(|line| {
            let line = line.trim();
            line == "write-all"
                || [
                    "actions",
                    "attestations",
                    "checks",
                    "contents",
                    "deployments",
                    "discussions",
                    "id-token",
                    "issues",
                    "packages",
                    "pages",
                    "pull-requests",
                    "repository-projects",
                    "security-events",
                    "statuses",
                ]
                .iter()
                .any(|permission| line == format!("{permission}: write"))
        })
    };
    if !has_permission(plan, "actions: read")
        || !has_permission(plan, "attestations: read")
        || !has_permission(plan, "contents: read")
        || !has_permission(apply, "actions: read")
        || !has_permission(apply, "attestations: read")
        || !has_permission(apply, "contents: read")
        || declares_write_scope(plan)
        || declares_write_scope(apply)
    {
        return Err("metadata recovery jobs must declare explicit read-only Actions, attestations, and contents permissions".into());
    }
    let execute = workflow_step_blocks(apply)
        .into_iter()
        .find(|(name, _)| {
            name == "Revalidate and apply only stable metadata with prebuilt Rust control"
        })
        .map(|(_, body)| body)
        .ok_or("metadata recovery apply step is missing its credential boundary")?;
    if !execute.contains("GH_TOKEN: ${{ github.token }}")
        || !execute
            .contains("V4_METADATA_WRITE_TOKEN: ${{ steps.metadata-app-token.outputs.token }}")
        || execute.contains("GH_TOKEN: ${{ steps.metadata-app-token.outputs.token }}")
    {
        return Err(
            "metadata recovery apply must keep read-only and metadata-write credentials separate"
                .into(),
        );
    }
    for marker in [
        "enum RecoveryTokenClass",
        "struct RecoveryCredentials",
        "fn select(&self, method: &str, endpoint: &str)",
        "RecoveryTokenClass::ReadOnly",
        "RecoveryTokenClass::MetadataWrite",
        "synthetic-read-only-token",
        "synthetic-metadata-write-token",
        "metadata_recovery_auth_routes_reads_and_one_stable_put_to_separate_tokens",
        "capture_owned_timeout_runtime_only",
        "capture_owned_timeout_to_file_runtime_only",
    ] {
        if !release.contains(marker) && !process.contains(marker) {
            return Err(format!(
                "metadata recovery token routing is missing `{marker}`"
            ));
        }
    }
    let transport = release
        .split_once("impl GhTransport {")
        .and_then(|(_, body)| body.split_once("\nimpl Transport for GhTransport"))
        .map(|(body, _)| body)
        .ok_or("metadata recovery GitHub transport is not bounded")?;
    if !transport.contains("capture_owned_timeout_runtime_only")
        || !transport.contains("capture_owned_timeout_to_file_runtime_only")
        || !transport.contains("let selected = self.selected_recovery_token(method, endpoint)?;")
        || !transport.contains("selected.map(|credential| credential.value)")
        || !transport.contains("recovery_credentials.is_some()")
    {
        return Err("metadata recovery GitHub children must use an operation-selected token and a cleared, bounded environment".into());
    }
    let transport_impl = release
        .split_once("impl Transport for GhTransport {")
        .and_then(|(_, body)| body.split_once("\nfn extract_http_status"))
        .map(|(body, _)| body)
        .ok_or("metadata recovery transport operations are not bounded")?;
    if !transport_impl.contains("self.selected_recovery_token(\"GET\", endpoint)?")
        || !transport_impl.contains("selected.map(|credential| credential.value)")
    {
        return Err(
            "GitHub asset downloads must use the read-only operation-selected token".into(),
        );
    }
    let attestation = recovery
        .split_once("impl ArtifactVerifier for GitHubArtifactVerifier")
        .and_then(|(_, body)| body.split_once("\npub(super) fn run"))
        .map(|(body, _)| body)
        .ok_or("metadata recovery attestation verifier is not bounded")?;
    if !attestation.contains("self.read_only_token")
        || !attestation.contains("capture_owned_timeout_runtime_only")
    {
        return Err(
            "attestation verification must use the isolated read-only workflow token".into(),
        );
    }
    let entrypoint = recovery
        .split_once("pub(super) fn run(root: &Path, command: &str, args: &[String])")
        .and_then(|(_, body)| body.split_once("\nfn current_plan_run()"))
        .map(|(body, _)| body)
        .ok_or("metadata recovery credential setup is not bounded")?;
    if !entrypoint.contains("env::var(\"GH_TOKEN\")")
        || !entrypoint.contains("env::var(super::METADATA_WRITE_TOKEN_ENV)")
        || !entrypoint.contains("GhTransport::for_metadata_recovery(")
        || !entrypoint.contains("GitHubArtifactVerifier::new(read_only_token)")
        || !entrypoint.contains("read-only recovery plan must not receive a metadata write token")
    {
        return Err("metadata recovery entrypoint must bind read and write credentials to their intended consumers".into());
    }
    let read_token_probe = process
        .split_once("fn capture_owned_timeout_runtime_only(")
        .and_then(|(_, body)| body.split_once("\nfn capture_owned_timeout_isolated"))
        .map(|(body, _)| body)
        .ok_or("runtime-only child environment helper is not bounded")?;
    let file_token_probe = process
        .split_once("pub(crate) fn capture_owned_timeout_to_file_runtime_only(")
        .and_then(|(_, body)| {
            body.split_once("\n#[cfg(windows)]\nfn capture_windows_timeout_to_file")
        })
        .map(|(body, _)| body)
        .ok_or("runtime-only file download helper is not bounded")?;
    if !read_token_probe.contains("runtime_environment_from(env::vars_os(), explicit_env)")
        || !file_token_probe.contains("runtime_environment_from(env::vars_os(), explicit_env)")
        || !file_token_probe.contains("clear_inherited: true")
        || !process.contains("V4_METADATA_WRITE_TOKEN")
        || !process.contains("synthetic-recovery-write-token")
        || !process.contains("runtime_child_environment_is_captured_after_secret_isolation")
    {
        return Err("recovery subprocess tests must prove the write credential is excluded from read-only children".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_recovery_auth_contract_rejects_permission_and_token_regressions() {
        let root = crate::repo::root();
        let workflow = fs::read_to_string(root.join(".github/workflows/v4-metadata-recovery.yml"))
            .expect("metadata recovery workflow");
        let release = fs::read_to_string(root.join("rust/xtask/src/release_v4.rs"))
            .expect("metadata recovery transport");
        let recovery = fs::read_to_string(root.join("rust/xtask/src/release_v4/recovery.rs"))
            .expect("metadata recovery control plane");
        let process = fs::read_to_string(root.join("rust/xtask/src/process.rs"))
            .expect("child process environment policy");
        metadata_recovery_auth_contract(&workflow, &release, &recovery, &process)
            .expect("metadata recovery tokens must remain separately scoped");

        let missing_plan_actions = workflow.replacen("      actions: read\n", "", 1);
        assert!(
            metadata_recovery_auth_contract(&missing_plan_actions, &release, &recovery, &process)
                .is_err()
        );

        let apply_has_write_scope = workflow.replace(
            "    environment: v4-production-release\n    permissions:\n      actions: read\n      attestations: read\n      contents: read\n    steps:",
            "    environment: v4-production-release\n    permissions:\n      actions: read\n      attestations: read\n      contents: read\n      checks: write\n    steps:",
        );
        assert_ne!(apply_has_write_scope, workflow);
        assert!(
            metadata_recovery_auth_contract(&apply_has_write_scope, &release, &recovery, &process)
                .is_err()
        );

        let app_token_used_for_reads = workflow.replace(
            "GH_TOKEN: ${{ github.token }}",
            "GH_TOKEN: ${{ steps.metadata-app-token.outputs.token }}",
        );
        assert!(
            metadata_recovery_auth_contract(
                &app_token_used_for_reads,
                &release,
                &recovery,
                &process
            )
            .is_err()
        );

        let attestation_uses_write_token = recovery.replace(
            "self.read_only_token.clone()",
            "std::env::var(super::METADATA_WRITE_TOKEN_ENV).unwrap()",
        );
        assert!(
            metadata_recovery_auth_contract(
                &workflow,
                &release,
                &attestation_uses_write_token,
                &process
            )
            .is_err()
        );

        let app_token_configures_attestations = recovery.replace(
            "GitHubArtifactVerifier::new(read_only_token)",
            "GitHubArtifactVerifier::new(metadata_write_token.unwrap())",
        );
        assert!(
            metadata_recovery_auth_contract(
                &workflow,
                &release,
                &app_token_configures_attestations,
                &process
            )
            .is_err()
        );

        let inherited_transport_environment = release.replace(
            "capture_owned_timeout_runtime_only(",
            "capture_owned_timeout_filtered(",
        );
        assert!(
            metadata_recovery_auth_contract(
                &workflow,
                &inherited_transport_environment,
                &recovery,
                &process
            )
            .is_err()
        );
    }
}
