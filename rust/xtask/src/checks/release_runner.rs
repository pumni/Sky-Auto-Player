use super::workflow::{workflow_declares_only_dispatch, workflow_job_blocks, workflow_step_blocks};
use crate::Result;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use walkdir::WalkDir;

const APPROVED_RELEASE_WORKFLOW: &str = ".github/workflows/release-v4.yml";
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
    for marker in [
        "cargo xtask release-v4 self-test",
        "cargo test --locked --manifest-path rust/xtask/Cargo.toml release_v4::tests",
        static_result_marker.as_str(),
        release_result_marker.as_str(),
        r#"if [[ "$STATIC_REQUIRED" == "true" ]]; then"#,
        r#"[[ "$STATIC_RESULT" == "success" ]]"#,
        r#"if [[ "$RELEASE_REQUIRED" == "true" ]]; then"#,
        r#"[[ "$RELEASE_CONTRACT_RESULT" == "success" ]]"#,
    ] {
        if !source.contains(marker) {
            return Err(format!(
                "CI is missing Rust release regression or status marker: {marker}"
            ));
        }
    }
    let marker = "needs: [changes, static, release_contract, supply_chain, validate, desktop_web, candidate, updater_bridge, updater_contract, updater_e2e, packaged, site]";
    if !status.contains(marker) {
        return Err(format!(
            "status gate is missing independent assertion: {marker}"
        ));
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
    const READ_MINT_STEP: &str = "Mint read-only metadata App token";
    const WRITE_MINT_STEP: &str = "Mint contents-scoped metadata App token";
    const PROBE_STEP: &str = "Probe release-metadata App access without write permission";
    const PROMOTE_STEP: &str = "Promote channel metadata after immutable release publication";
    const APP_TOKEN_OUTPUT: &str = "steps.metadata-app-token.outputs.token";
    const APP_PRIVATE_KEY: &str = "secrets.V4_RELEASE_METADATA_APP_PRIVATE_KEY";

    let steps = workflow_step_blocks(workflow);
    let read_mint = steps
        .iter()
        .find(|(name, _)| name == READ_MINT_STEP)
        .map(|(_, block)| block.as_str())
        .ok_or("v4 release workflow is missing the read-only metadata App token mint step")?;
    let write_mint = steps
        .iter()
        .find(|(name, _)| name == WRITE_MINT_STEP)
        .map(|(_, block)| block.as_str())
        .ok_or("v4 release workflow is missing the write-scoped metadata App token mint step")?;
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
        (read_mint, "permission-contents: read"),
        (write_mint, "permission-contents: write"),
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
            "metadata App token must be consumed by the read-only probe and PromoteMetadata".into(),
        );
    }
    if probe.contains("GH_TOKEN: ${{ github.token }}") {
        return Err("metadata App access probe must not use the repository GITHUB_TOKEN".into());
    }
    if !probe.contains("cargo xtask release-v4 probe-metadata-token")
        || probe.contains("probe_v4_metadata_app_access.ps1")
    {
        return Err("metadata App access probe must use the Rust read-only token probe".into());
    }
    if promote.contains("GH_TOKEN: ${{ github.token }}") {
        return Err("PromoteMetadata must not use the repository GITHUB_TOKEN".into());
    }

    for (name, block) in &steps {
        if name != READ_MINT_STEP && name != WRITE_MINT_STEP && block.contains(APP_PRIVATE_KEY) {
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
    {
        return Err(
            "metadata App read access must be proven before the publication job starts".into(),
        );
    }

    Ok(())
}

pub(super) fn validate_release_runner_workflow(relative: &str, source: &str) -> Result<()> {
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

pub(super) fn release_runner_contract(root: &Path) -> Result<()> {
    let ci = fs::read_to_string(root.join(".github/workflows/ci.yml"))?;
    v4_release_contract_ci_wiring(&ci)
        .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> { error.into() })?;
    let release = fs::read_to_string(root.join(".github/workflows/release-v4.yml"))?;
    validate_metadata_app_token_scope(&release)?;

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
        if relative == APPROVED_RELEASE_WORKFLOW {
            observed.insert(relative);
        }
    }
    if !observed.contains(APPROVED_RELEASE_WORKFLOW) {
        return Err(format!(
            "canonical production workflow is missing: {APPROVED_RELEASE_WORKFLOW}"
        )
        .into());
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

        let pull_request = workflow.replace(
            "  workflow_dispatch:\n",
            "  workflow_dispatch:\n  pull_request:\n",
        );
        assert!(
            validate_release_runner_workflow(APPROVED_RELEASE_WORKFLOW, &pull_request).is_err()
        );

        let unprotected = workflow.replacen("    environment: v4-production-release\n", "", 1);
        assert!(validate_release_runner_workflow(APPROVED_RELEASE_WORKFLOW, &unprotected).is_err());

        let broad_runner =
            workflow.replace("runs-on: windows-latest", "runs-on: [self-hosted, windows]");
        assert!(
            validate_release_runner_workflow(APPROVED_RELEASE_WORKFLOW, &broad_runner).is_err()
        );

        let missing_secret = workflow.replace(
            "          V4_TAURI_UPDATER_PRIVATE_KEY: ${{ secrets.V4_TAURI_UPDATER_PRIVATE_KEY }}\n",
            "",
        );
        assert!(
            validate_release_runner_workflow(APPROVED_RELEASE_WORKFLOW, &missing_secret).is_err()
        );
    }

    #[test]
    fn hosted_workflow_documentation_matches_the_single_release_path() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let documentation = fs::read_to_string(root.join("docs/v4-release-execution-topology.md"))
            .expect("release topology documentation must exist");
        release_runner_documentation_contract_source(&documentation)
            .expect("documentation must list the canonical production workflow");
        let missing = documentation.replace(
            "`.github/workflows/release-v4.yml`",
            "`production-workflow-omitted.yml`",
        );
        assert!(release_runner_documentation_contract_source(&missing).is_err());
        let mut with_extra = documentation.clone();
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
        assert!(release_runner_documentation_contract_source(&with_extra).is_err());
    }

    #[test]
    fn metadata_app_tokens_are_separate_and_only_reach_probe_or_promotion() {
        let workflow = current_workflow();
        assert!(validate_metadata_app_token_scope(&workflow).is_ok());

        let stale_pin = workflow.replace(
            "uses: actions/create-github-app-token@64ab729a6294000f5aa4a4bc0b73d1f32bc45510",
            "uses: actions/create-github-app-token@bcd2ba49218906704ab6c1aa796996da409d3eb1",
        );
        assert!(validate_metadata_app_token_scope(&stale_pin).is_err());

        let missing_client_id = workflow.replace(
            "          client-id: ${{ vars.V4_RELEASE_METADATA_APP_CLIENT_ID }}\n",
            "",
        );
        assert!(validate_metadata_app_token_scope(&missing_client_id).is_err());

        let missing_read_scope =
            workflow.replace("permission-contents: read", "permission-contents: write");
        assert!(validate_metadata_app_token_scope(&missing_read_scope).is_err());

        let broad_token = workflow.replace(
            "- name: Publish the exact qualified candidate immutably\n        env:\n          GH_TOKEN: ${{ github.token }}",
            "- name: Publish the exact qualified candidate immutably\n        env:\n          GH_TOKEN: ${{ steps.metadata-app-token.outputs.token }}",
        );
        assert!(validate_metadata_app_token_scope(&broad_token).is_err());

        let private_key_in_env = workflow.replace(
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
"#.to_owned();
        assert!(v4_release_contract_ci_wiring(&source).is_ok());
        for marker in [
            "cargo xtask release-v4 self-test",
            "cargo test --locked --manifest-path rust/xtask/Cargo.toml release_v4::tests",
        ] {
            assert!(v4_release_contract_ci_wiring(&source.replace(marker, "removed")).is_err());
        }
        let no_static = source.replace(
            "cargo xtask check static --skip-supply-chain",
            "cargo xtask check static",
        );
        assert!(v4_release_contract_ci_wiring(&no_static).is_err());
        let production_api = source.replace(
            "cargo xtask release-v4 self-test",
            "gh api repos/pumni/Sky-Auto-Player/releases",
        );
        assert!(v4_release_contract_ci_wiring(&production_api).is_err());
    }
}
