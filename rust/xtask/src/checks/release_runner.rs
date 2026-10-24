use super::workflow::{workflow_declares_only_dispatch, workflow_job_blocks, workflow_step_blocks};
use crate::Result;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use walkdir::WalkDir;

const APPROVED_RELEASE_RUNNER_WORKFLOWS: &[&str] = &[
    ".github/workflows/release-v4.yml",
    ".github/workflows/rehearse-v4.yml",
];
pub(super) const RELEASE_RUNNER_LABEL_MARKER: &str =
    "runs-on: [self-hosted, windows, v4-release, single-tenant]";

fn approved_release_runner_job(relative: &str) -> Option<&'static str> {
    match relative {
        ".github/workflows/release-v4.yml" => Some("release"),
        ".github/workflows/rehearse-v4.yml" => Some("qualification"),
        _ => None,
    }
}

pub(super) fn v4_release_contract_ci_wiring(source: &str) -> std::result::Result<(), String> {
    let jobs = workflow_job_blocks(source);
    let static_job = jobs
        .iter()
        .find(|(job_id, _)| job_id == "static")
        .map(|(_, block)| block)
        .ok_or_else(|| "CI is missing the static job block".to_owned())?;
    let release_contract = jobs
        .iter()
        .find(|(job_id, _)| job_id == "release_contract")
        .map(|(_, block)| block)
        .ok_or_else(|| "CI is missing the release_contract job block".to_owned())?;
    let status = jobs
        .iter()
        .find(|(job_id, _)| job_id == "status")
        .map(|(_, block)| block)
        .ok_or_else(|| "CI is missing the status job block".to_owned())?;
    const FULL_STATIC_CHECK: &str = "cargo xtask check static --skip-supply-chain";
    if static_job.matches(FULL_STATIC_CHECK).count() != 1 {
        return Err(
            "static job must contain exactly one full static verification invocation".to_owned(),
        );
    }
    if release_contract.contains(FULL_STATIC_CHECK) {
        return Err("release_contract must not repeat the full static verification".to_owned());
    }
    if !release_contract
        .lines()
        .any(|line| line.trim() == "needs: changes")
    {
        return Err("release_contract must remain directly parallel with static".to_owned());
    }
    if release_contract.lines().any(|line| {
        let trimmed = line.trim();
        trimmed.starts_with("needs:") && trimmed.contains("static")
    }) {
        return Err("release_contract must not depend on static".to_owned());
    }
    for marker in [
        "needs: [changes, static, release_contract, supply_chain, validate, desktop_web, candidate, updater_bridge, updater_contract, updater_e2e, packaged, site]",
        "STATIC_RESULT: ${{ needs.static.result }}",
        "RELEASE_CONTRACT_RESULT: ${{ needs.release_contract.result }}",
        r#"if [[ "$STATIC_REQUIRED" == "true" ]]; then"#,
        r#"[[ "$STATIC_RESULT" == "success" ]]"#,
        r#"if [[ "$RELEASE_REQUIRED" == "true" ]]; then"#,
        r#"[[ "$RELEASE_CONTRACT_RESULT" == "success" ]]"#,
    ] {
        if !status.contains(marker) {
            return Err(format!(
                "status gate is missing its independent static/release assertion: {marker}"
            ));
        }
    }
    for marker in [
        "name: Run V4 release Latest guard behavioral test",
        "pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass -File scripts/test_ci_v4_release_latest_guard.ps1",
        "name: Run V4 post-publication transaction behavioral test",
        "pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass -File scripts/test_v4_post_publication_transaction.ps1",
    ] {
        if !release_contract.contains(marker) {
            return Err(format!(
                "release_contract job is missing the behavioral regression marker: {marker}"
            ));
        }
    }
    Ok(())
}

pub(super) fn validate_metadata_app_token_scope(workflow: &str) -> Result<()> {
    const MINT_STEP: &str = "Mint release-metadata GitHub App token before publication";
    const PROBE_STEP: &str = "Probe release-metadata App access before publication";
    const PROMOTE_STEP: &str = "Promote release metadata only after immutable publication";
    const APP_TOKEN_OUTPUT: &str = "steps.metadata-app-token.outputs.token";
    const APP_PRIVATE_KEY: &str = "secrets.V4_RELEASE_METADATA_APP_PRIVATE_KEY";

    let steps = workflow_step_blocks(workflow);
    let mint = steps
        .iter()
        .find(|(name, _)| name == MINT_STEP)
        .map(|(_, block)| block.as_str())
        .ok_or("v4 release workflow is missing the metadata App token mint step")?;
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

    for marker in [
        "id: metadata-app-token",
        "uses: actions/create-github-app-token@64ab729a6294000f5aa4a4bc0b73d1f32bc45510",
        "client-id: ${{ vars.V4_RELEASE_METADATA_APP_CLIENT_ID }}",
        "private-key: ${{ secrets.V4_RELEASE_METADATA_APP_PRIVATE_KEY }}",
        "owner: ${{ github.repository_owner }}",
        "repositories: ${{ github.event.repository.name }}",
        "permission-contents: write",
    ] {
        if !mint.contains(marker) {
            return Err(format!(
                "metadata App token mint step is missing required marker: {marker}"
            )
            .into());
        }
    }
    if mint.contains("\n        run:") || mint.contains("\n        env:") {
        return Err(
            "metadata App private key must be consumed only by the token-mint action".into(),
        );
    }
    if workflow.matches(APP_PRIVATE_KEY).count() != 1 {
        return Err(
            "metadata App private key must occur exactly once in the release workflow".into(),
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
            "metadata App token must be consumed by the read-only probe and PromoteMetadata".into(),
        );
    }
    if probe.contains("GH_TOKEN: ${{ github.token }}") {
        return Err("metadata App access probe must not use the repository GITHUB_TOKEN".into());
    }
    if promote.contains("GH_TOKEN: ${{ github.token }}") {
        return Err("PromoteMetadata must not use the repository GITHUB_TOKEN".into());
    }

    for (name, block) in &steps {
        if name != MINT_STEP && block.contains(APP_PRIVATE_KEY) {
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
        "Preflight release request and repository readiness",
        "Publish the qualified candidate immutably",
        "Re-fetch and verify final public release and metadata",
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

    let mint_position = workflow
        .find("- name: Mint release-metadata GitHub App token before publication")
        .ok_or("metadata App token mint step position is unavailable")?;
    let probe_position = workflow
        .find("- name: Probe release-metadata App access before publication")
        .ok_or("metadata App access probe position is unavailable")?;
    let publish_position = workflow
        .find("- name: Publish the qualified candidate immutably")
        .ok_or("publication step position is unavailable")?;
    let promote_position = workflow
        .find("- name: Promote release metadata only after immutable publication")
        .ok_or("metadata promotion step position is unavailable")?;
    if mint_position >= probe_position
        || probe_position >= publish_position
        || publish_position >= promote_position
    {
        return Err(
            "metadata App token must be minted and probed before immutable publication".into(),
        );
    }

    Ok(())
}

fn job_uses_sensitive_runner(job: &str) -> bool {
    job.lines().any(|line| {
        let trimmed = line.trim_start();
        trimmed.starts_with("runs-on:")
            && (trimmed.contains("self-hosted")
                || trimmed.contains("v4-release")
                || trimmed.contains("single-tenant"))
    })
}

fn job_has_exact_release_runner_labels(job: &str) -> bool {
    job.lines()
        .any(|line| line.trim() == RELEASE_RUNNER_LABEL_MARKER)
}

fn job_has_protected_release_environment(job: &str) -> bool {
    job.lines()
        .any(|line| line.trim() == "environment: v4-production-release")
}

pub(super) fn validate_release_runner_workflow(relative: &str, source: &str) -> Result<()> {
    let jobs = workflow_job_blocks(source);
    let sensitive_jobs = jobs
        .iter()
        .filter(|(_, job)| job_uses_sensitive_runner(job))
        .collect::<Vec<_>>();
    let source_mentions_sensitive_runner = source.contains("self-hosted")
        || source.contains("v4-release")
        || source.contains("single-tenant");
    if !source_mentions_sensitive_runner {
        return Ok(());
    }
    if !APPROVED_RELEASE_RUNNER_WORKFLOWS.contains(&relative) {
        return Err(format!(
            "unapproved workflow targets the production release runner boundary: {relative}"
        )
        .into());
    }
    if !workflow_declares_only_dispatch(source) {
        return Err(format!(
            "production release runner workflow must whitelist workflow_dispatch as its only trigger: {relative}"
        )
        .into());
    }
    let approved_job = approved_release_runner_job(relative).ok_or_else(|| {
        format!("approved release runner workflow has no approved job mapping: {relative}")
    })?;
    if sensitive_jobs.is_empty() {
        return Err(format!(
            "release runner labels could not be mapped to a job block: {relative}"
        )
        .into());
    }
    for (job_id, job) in &sensitive_jobs {
        if job_id != approved_job {
            return Err(format!(
                "job `{job_id}` is not approved for the production release runner boundary: {relative}"
            )
            .into());
        }
        if !job_has_exact_release_runner_labels(job) {
            return Err(format!(
                "job `{job_id}` must use the exact dedicated release runner labels: {relative}"
            )
            .into());
        }
        if !job_has_protected_release_environment(job) {
            return Err(format!(
                "job `{job_id}` must declare environment: v4-production-release in its own job block: {relative}"
            )
            .into());
        }
    }
    if sensitive_jobs.len() != 1 {
        return Err(format!(
            "approved release runner workflow must contain exactly one sensitive runner job: {relative}"
        )
        .into());
    }
    for marker in [
        "workflow_dispatch:",
        "dispatch-boundary",
        "github.event.repository.default_branch",
        "refs/heads/main",
        "V4_UPDATER_PRIVATE_KEY_PATH",
        "RUNNER_TEMP",
        "verify_v4_release_runner.ps1",
        "cleanup_v4_release_state.ps1",
        "persist-credentials: false",
    ] {
        if !source.contains(marker) {
            return Err(format!(
                "approved release runner workflow is missing its trust-boundary marker `{marker}`: {relative}"
            )
            .into());
        }
    }
    for forbidden_input in [
        "updater_private_key_path:",
        "inputs.updater_private_key_path",
    ] {
        if source.contains(forbidden_input) {
            return Err(format!(
                "production release runner workflow accepts an operator-controlled key path `{forbidden_input}`: {relative}"
            )
            .into());
        }
    }
    Ok(())
}

pub(super) fn release_runner_contract(root: &Path) -> Result<()> {
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
        if APPROVED_RELEASE_RUNNER_WORKFLOWS.contains(&relative.as_str()) {
            observed.insert(relative);
        }
    }
    for required in APPROVED_RELEASE_RUNNER_WORKFLOWS {
        if !observed.contains(*required) {
            return Err(format!("approved release runner workflow is missing: {required}").into());
        }
    }
    release_runner_documentation_contract(root)?;
    println!("[xtask] v4 release runner trust boundary: PASS");
    Ok(())
}

fn release_runner_documentation_contract(root: &Path) -> Result<()> {
    let documentation = fs::read_to_string(root.join("docs/v4-release-execution-topology.md"))?;
    release_runner_documentation_contract_source(&documentation)
}

fn release_runner_documentation_contract_source(documentation: &str) -> Result<()> {
    let section_start = documentation
        .find("### 2.4 GitHub Actions Runner Isolation and Operator Requirements")
        .ok_or("release runner documentation is missing section 2.4")?;
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
    let expected = APPROVED_RELEASE_RUNNER_WORKFLOWS
        .iter()
        .map(|workflow| (*workflow).to_owned())
        .collect::<BTreeSet<_>>();
    if documented != expected {
        return Err(format!(
            "documented release runner workflow allowlist does not match the enforced allowlist: documented={documented:?}, enforced={expected:?}"
        )
        .into());
    }
    for workflow in APPROVED_RELEASE_RUNNER_WORKFLOWS {
        let marker = format!("`{workflow}`");
        if section.matches(&marker).count() != 1 {
            return Err(format!(
                "documented release runner workflow must appear exactly once in section 2.4: {workflow}"
            )
            .into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_runner_contract_rejects_sensitive_runner_on_general_ci() {
        let workflow = r#"
on:
  pull_request:
jobs:
  build:
    runs-on: [self-hosted, windows, v4-release, single-tenant]
"#;
        let error = validate_release_runner_workflow(".github/workflows/ci.yml", workflow)
            .expect_err("general CI must not target the production signing runner");
        assert!(error.to_string().contains("unapproved workflow"));
    }

    #[test]
    fn release_runner_contract_rejects_untrusted_trigger_on_approved_workflow() {
        let workflow = r#"
on:
  workflow_dispatch:
  pull_request:
jobs:
  release:
    runs-on: [self-hosted, windows, v4-release, single-tenant]
"#;
        let error = validate_release_runner_workflow(".github/workflows/release-v4.yml", workflow)
            .expect_err("approved release workflow must reject PR triggers");
        assert!(error.to_string().contains("only trigger"));
    }

    #[test]
    fn release_runner_contract_rejects_unprotected_sensitive_job_subset() {
        let workflow = r#"
on:
  workflow_dispatch:
jobs:
  release:
    runs-on: [self-hosted, windows, v4-release, single-tenant]
    environment: v4-production-release
  unprotected:
    runs-on: [self-hosted, windows, v4-release]
"#;
        let error = validate_release_runner_workflow(".github/workflows/release-v4.yml", workflow)
            .expect_err("an unprotected sensitive job subset must be rejected");
        assert!(error.to_string().contains("not approved"));
    }

    #[test]
    fn release_runner_documentation_allowlist_matches_enforced_workflows() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let documentation = fs::read_to_string(root.join("docs/v4-release-execution-topology.md"))
            .expect("release runner documentation fixture must exist");
        release_runner_documentation_contract_source(&documentation)
            .expect("documentation must enumerate exactly the approved runner workflows");

        let missing = documentation.replace(
            "`.github/workflows/rehearse-v4.yml`",
            "`qualification-workflow-omitted.yml`",
        );
        assert!(
            release_runner_documentation_contract_source(&missing).is_err(),
            "documentation must fail when an approved runner workflow is omitted"
        );

        let extra = documentation.replace(
            "- `.github/workflows/release-v4.yml` — job `release`, the official immutable publication path.",
            "- `.github/workflows/release-v4.yml` — job `release`, the official immutable publication path.\n- `.github/workflows/unapproved.yml` — not approved.",
        );
        assert!(
            release_runner_documentation_contract_source(&extra).is_err(),
            "documentation must fail when an unapproved runner workflow is added"
        );
    }

    #[test]
    fn metadata_app_token_is_scoped_to_promotion_and_keeps_private_key_in_action_input() {
        let workflow = r#"
      - name: Preflight release request and repository readiness
        env:
          GH_TOKEN: ${{ github.token }}
      - name: Mint release-metadata GitHub App token before publication
        id: metadata-app-token
        uses: actions/create-github-app-token@64ab729a6294000f5aa4a4bc0b73d1f32bc45510
        with:
          client-id: ${{ vars.V4_RELEASE_METADATA_APP_CLIENT_ID }}
          private-key: ${{ secrets.V4_RELEASE_METADATA_APP_PRIVATE_KEY }}
          owner: ${{ github.repository_owner }}
          repositories: ${{ github.event.repository.name }}
          permission-contents: write
      - name: Probe release-metadata App access before publication
        env:
          GH_TOKEN: ${{ steps.metadata-app-token.outputs.token }}
        run: scripts/probe_v4_metadata_app_access.ps1
      - name: Publish the qualified candidate immutably
        env:
          GH_TOKEN: ${{ github.token }}
      - name: Promote release metadata only after immutable publication
        env:
          GH_TOKEN: ${{ steps.metadata-app-token.outputs.token }}
      - name: Re-fetch and verify final public release and metadata
        env:
          GH_TOKEN: ${{ github.token }}
"#;
        assert!(validate_metadata_app_token_scope(workflow).is_ok());

        let stale_pin = workflow.replace(
            "uses: actions/create-github-app-token@64ab729a6294000f5aa4a4bc0b73d1f32bc45510",
            "uses: actions/create-github-app-token@bcd2ba49218906704ab6c1aa796996da409d3eb1",
        );
        assert_ne!(stale_pin, workflow);
        assert!(
            validate_metadata_app_token_scope(&stale_pin).is_err(),
            "metadata App token contract must reject the previous immutable SHA"
        );

        let missing_client_id = workflow.replace(
            "          client-id: ${{ vars.V4_RELEASE_METADATA_APP_CLIENT_ID }}\n",
            "",
        );
        assert!(
            validate_metadata_app_token_scope(&missing_client_id).is_err(),
            "metadata App token contract must reject a missing client-id"
        );

        let deprecated_app_id = workflow.replace(
            "client-id: ${{ vars.V4_RELEASE_METADATA_APP_CLIENT_ID }}",
            "app-id: ${{ vars.V4_RELEASE_METADATA_APP_ID }}",
        );
        assert!(
            validate_metadata_app_token_scope(&deprecated_app_id).is_err(),
            "metadata App token contract must reject deprecated app-id"
        );

        let deprecated_client_id_variable = workflow.replace(
            "client-id: ${{ vars.V4_RELEASE_METADATA_APP_CLIENT_ID }}",
            "client-id: ${{ vars.V4_RELEASE_METADATA_APP_ID }}",
        );
        assert!(
            validate_metadata_app_token_scope(&deprecated_client_id_variable).is_err(),
            "metadata App token contract must reject the deprecated App ID variable"
        );

        let broad_token = workflow.replace(
            "- name: Publish the qualified candidate immutably\n        env:\n          GH_TOKEN: ${{ github.token }}",
            "- name: Publish the qualified candidate immutably\n        env:\n          GH_TOKEN: ${{ steps.metadata-app-token.outputs.token }}",
        );
        assert!(validate_metadata_app_token_scope(&broad_token).is_err());

        let repository_token_for_promotion = workflow.replace(
            "GH_TOKEN: ${{ steps.metadata-app-token.outputs.token }}",
            "GH_TOKEN: ${{ github.token }}",
        );
        assert!(validate_metadata_app_token_scope(&repository_token_for_promotion).is_err());

        let private_key_in_env = workflow.replace(
            "        with:\n          client-id:",
            "        env:\n          client-id:",
        );
        assert!(validate_metadata_app_token_scope(&private_key_in_env).is_err());
    }

    #[test]
    fn release_contract_ci_wiring_requires_both_behavioral_regressions() {
        let source = r#"
jobs:
  static:
    needs: changes
    steps:
      - run: cargo xtask check static --skip-supply-chain
  release_contract:
    name: V4 release contract acceptance
    needs: changes
    steps:
      - name: Run V4 release Latest guard behavioral test
        run: pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass -File scripts/test_ci_v4_release_latest_guard.ps1
      - name: Run V4 post-publication transaction behavioral test
        run: pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass -File scripts/test_v4_post_publication_transaction.ps1
  status:
    needs: [changes, static, release_contract, supply_chain, validate, desktop_web, candidate, updater_bridge, updater_contract, updater_e2e, packaged, site]
    env:
      STATIC_RESULT: ${{ needs.static.result }}
      RELEASE_CONTRACT_RESULT: ${{ needs.release_contract.result }}
    run: |
      if [[ "$STATIC_REQUIRED" == "true" ]]; then
        [[ "$STATIC_RESULT" == "success" ]]
      fi
      if [[ "$RELEASE_REQUIRED" == "true" ]]; then
        [[ "$RELEASE_CONTRACT_RESULT" == "success" ]]
      fi
"#;
        assert!(v4_release_contract_ci_wiring(source).is_ok());
        let without_static_check = source.replace(
            "cargo xtask check static --skip-supply-chain",
            "cargo xtask check static",
        );
        assert!(v4_release_contract_ci_wiring(&without_static_check).is_err());
        let duplicated_release_check = source.replace(
            "      - name: Run V4 release Latest guard behavioral test",
            "      - run: cargo xtask check static --skip-supply-chain\n      - name: Run V4 release Latest guard behavioral test",
        );
        assert!(v4_release_contract_ci_wiring(&duplicated_release_check).is_err());
        let serialized_release_contract = source.replace(
            "    needs: changes\n    steps:\n      - name: Run V4 release Latest guard behavioral test",
            "    needs: static\n    steps:\n      - name: Run V4 release Latest guard behavioral test",
        );
        assert!(v4_release_contract_ci_wiring(&serialized_release_contract).is_err());
        let without_static_status_assertion = source.replace(
            "STATIC_RESULT: ${{ needs.static.result }}",
            "STATIC_RESULT: ${{ needs.other.result }}",
        );
        assert!(v4_release_contract_ci_wiring(&without_static_status_assertion).is_err());
        for marker in [
            "scripts/test_ci_v4_release_latest_guard.ps1",
            "scripts/test_v4_post_publication_transaction.ps1",
        ] {
            let without_marker = source.replace(marker, "scripts/removed_behavioral_test.ps1");
            assert!(
                v4_release_contract_ci_wiring(&without_marker).is_err(),
                "removing {marker} must fail the release contract wiring"
            );
        }
    }
}
