#![forbid(unsafe_code)]

use crate::{Result, ci_artifacts};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) const LANES: [&str; 10] = [
    "rust_required",
    "desktop_required",
    "desktop_native_required",
    "desktop_e2e_required",
    "package_required",
    "updater_required",
    "release_required",
    "signing_proof_required",
    "supply_chain_required",
    "site_required",
];

const SHALLOW_DEEPEN_STEPS: [usize; 4] = [32, 128, 512, 2048];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DiffMode {
    PullRequest,
    Push,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PullRequestRefs {
    number: u64,
    base_ref: String,
}

impl PullRequestRefs {
    fn parse(number: &str, base_ref: String) -> Result<Self> {
        let number = number
            .parse::<u64>()
            .ok()
            .filter(|number| *number > 0)
            .ok_or("pull request number must be a positive integer")?;
        if base_ref.is_empty() {
            return Err("pull request base ref is required".into());
        }
        Ok(Self { number, base_ref })
    }

    fn fetch_refspecs(&self, root: &Path) -> Result<(String, String)> {
        let base = format!("refs/heads/{}", self.base_ref);
        let valid = git_output(root, &["check-ref-format", base.as_str()])?;
        if !valid.status.success() {
            return Err("pull request base ref is not a valid branch name".into());
        }
        Ok((
            format!("+{base}:refs/sky-ci/base"),
            format!("+refs/pull/{}/head:refs/sky-ci/pr-head", self.number),
        ))
    }
}

impl DiffMode {
    fn parse(value: &str) -> Result<Self> {
        match value {
            "pr" => Ok(Self::PullRequest),
            "push" => Ok(Self::Push),
            _ => Err(format!("unsupported diff mode: {value}; expected pr or push").into()),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Classification {
    lanes: [bool; LANES.len()],
    pub(crate) classification_reason: String,
}

impl Classification {
    fn new(reason: impl Into<String>) -> Self {
        Self {
            lanes: [false; LANES.len()],
            classification_reason: reason.into(),
        }
    }

    fn full(reason: impl Into<String>) -> Self {
        Self {
            lanes: [true; LANES.len()],
            classification_reason: reason.into(),
        }
    }

    fn add(&mut self, lane: &str) {
        if let Some(index) = LANES.iter().position(|candidate| *candidate == lane) {
            self.lanes[index] = true;
        }
    }

    fn add_many(&mut self, lanes: &[&str]) {
        for lane in lanes {
            self.add(lane);
        }
    }

    pub(crate) fn lane(&self, name: &str) -> bool {
        LANES
            .iter()
            .position(|candidate| *candidate == name)
            .is_some_and(|index| self.lanes[index])
    }

    pub(crate) fn render(&self) -> String {
        let mut lines = LANES
            .iter()
            .enumerate()
            .map(|(index, name)| format!("{name}={}", self.lanes[index]))
            .collect::<Vec<_>>();
        lines.push(format!(
            "classification_reason={}",
            self.classification_reason
        ));
        lines.join("\n")
    }
}

fn display_paths(paths: &[String]) -> String {
    paths
        .iter()
        .take(3)
        .map(|path| {
            path.chars()
                .flat_map(char::escape_default)
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn normalize_manual_path(raw: &str) -> String {
    let mut path = raw.trim().replace('\\', "/");
    while path.starts_with("./") {
        path.drain(..2);
    }
    path
}

fn classify_path(
    path: &str,
    classification: &mut Classification,
    unknown: &mut Vec<String>,
    static_only: &mut Vec<String>,
    mode: DiffMode,
) {
    if path == "README.md"
        || path == "CHANGELOG.md"
        || (path.starts_with("docs/") && !path.starts_with("docs/releases/"))
    {
        return;
    }

    if path.starts_with("docs/releases/") {
        static_only.push(path.to_owned());
        return;
    }

    if path == ".bun-version" {
        classification.add_many(&LANES);
        return;
    }

    if [
        "scripts/toolchain-pin.mjs",
        ".github/actions/setup-rust-pinned/action.yml",
    ]
    .contains(&path)
    {
        classification.add_many(&LANES);
        return;
    }

    if path.starts_with("apps/website/")
        || path.starts_with(".github/actions/site-validate/")
        || path == ".github/workflows/pages.yml"
    {
        classification.add("site_required");
        return;
    }

    if path == ".github/workflows/ci.yml"
        || (path.starts_with(".github/workflows/")
            && ![
                ".github/workflows/pages.yml",
                ".github/workflows/release-v4.yml",
            ]
            .contains(&path))
        || (path.starts_with(".github/actions/")
            && !path.starts_with(".github/actions/site-validate/"))
    {
        unknown.push(path.to_owned());
        return;
    }

    if path == ".github/workflows/release-v4.yml" {
        classification.add_many(&["release_required", "signing_proof_required"]);
        return;
    }

    if path == ".config/rust_architecture_allowlist.json" {
        static_only.push(path.to_owned());
        return;
    }
    if path == ".config/security_audit_baseline.json" {
        classification.add("supply_chain_required");
        return;
    }

    if [
        "xtask/src/release_signing.rs",
        "xtask/src/authenticode.rs",
        "xtask/src/updater_trust.rs",
    ]
    .contains(&path)
        || path.starts_with("xtask/src/release_signing/")
    {
        classification.add_many(&[
            "rust_required",
            "package_required",
            "updater_required",
            "release_required",
            "signing_proof_required",
        ]);
        return;
    }
    if path.starts_with("xtask/src/checks/") {
        unknown.push(path.to_owned());
        return;
    }

    if path == "apps/desktop/src-tauri/Cargo.toml" {
        classification.add_many(&[
            "rust_required",
            "desktop_required",
            "desktop_native_required",
            "package_required",
            "updater_required",
            "release_required",
            "supply_chain_required",
            "site_required",
        ]);
        return;
    }
    if path == "apps/desktop/src-tauri/src/native_update.rs" {
        classification.add_many(&[
            "rust_required",
            "desktop_required",
            "desktop_native_required",
            "package_required",
            "updater_required",
        ]);
        return;
    }
    if [
        "apps/desktop/src-tauri/src/main.rs",
        "apps/desktop/src-tauri/src/lib.rs",
    ]
    .contains(&path)
    {
        classification.add_many(&[
            "rust_required",
            "desktop_required",
            "desktop_native_required",
            "package_required",
        ]);
        return;
    }
    if path == "apps/desktop/src-tauri/build.rs" {
        classification.add_many(&[
            "rust_required",
            "desktop_required",
            "desktop_native_required",
            "package_required",
            "updater_required",
        ]);
        return;
    }
    if path == "apps/desktop/src-tauri/tauri.conf.json" {
        classification.add_many(&[
            "desktop_required",
            "desktop_native_required",
            "package_required",
            "updater_required",
            "signing_proof_required",
        ]);
        return;
    }
    if path == "apps/desktop/src-tauri/nsis-hooks.nsh"
        || path.starts_with("apps/desktop/src-tauri/resources/")
        || path.ends_with(".nsi")
        || path.ends_with(".nsh")
    {
        classification.add_many(&[
            "desktop_native_required",
            "package_required",
            "signing_proof_required",
        ]);
        return;
    }
    if path.starts_with("apps/desktop/src-tauri/capabilities/") {
        classification.add_many(&[
            "desktop_required",
            "desktop_native_required",
            "package_required",
        ]);
        return;
    }
    if path.starts_with("apps/desktop/src-tauri/icons/") {
        classification.add("package_required");
        return;
    }
    if path.starts_with("apps/desktop/src-tauri/src/") {
        classification.add_many(&[
            "rust_required",
            "desktop_required",
            "desktop_native_required",
            "package_required",
        ]);
        return;
    }
    if is_windows_integration_frontend_path(path) {
        classification.add_many(&[
            "desktop_required",
            "desktop_native_required",
            "desktop_e2e_required",
            "package_required",
        ]);
        return;
    }
    if ["apps/desktop/package.json", "apps/desktop/bun.lock"].contains(&path) {
        classification.add_many(&[
            "desktop_required",
            "desktop_e2e_required",
            "package_required",
            "updater_required",
            "supply_chain_required",
        ]);
        return;
    }
    if path.starts_with("apps/desktop/") {
        classification.add_many(&["desktop_required", "desktop_e2e_required"]);
        if mode == DiffMode::Push {
            classification.add("package_required");
        }
        return;
    }

    if ["Cargo.toml", "Cargo.lock"].contains(&path) {
        classification.add_many(&[
            "rust_required",
            "package_required",
            "updater_required",
            "release_required",
            "supply_chain_required",
        ]);
        return;
    }
    if [
        "crates/sky_windows_security/src/process_tree.rs",
        "crates/sky_windows_security/src/registry.rs",
        "crates/sky_windows_security/src/win32.rs",
    ]
    .contains(&path)
    {
        classification.add_many(&[
            "rust_required",
            "desktop_native_required",
            "package_required",
            "release_required",
        ]);
        return;
    }
    if path == "rust-toolchain.toml" || path.starts_with(".cargo/") {
        classification.add_many(&[
            "rust_required",
            "desktop_required",
            "desktop_native_required",
            "package_required",
            "updater_required",
            "release_required",
        ]);
        return;
    }
    if path.starts_with("crates/") {
        classification.add_many(&[
            "rust_required",
            "desktop_native_required",
            "package_required",
        ]);
        if is_crate_manifest(path) {
            classification.add("supply_chain_required");
        }
        return;
    }
    if [
        "xtask/src/ci_classifier.rs",
        "xtask/src/ci.rs",
        "xtask/src/main.rs",
    ]
    .contains(&path)
    {
        unknown.push(path.to_owned());
        return;
    }
    if [
        "xtask/src/ci_artifacts.rs",
        "xtask/src/ci_candidate.rs",
        "xtask/src/ci_bridge.rs",
        "xtask/src/startup_calibration.rs",
    ]
    .contains(&path)
    {
        classification.add_many(&[
            "rust_required",
            "desktop_native_required",
            "package_required",
            "updater_required",
        ]);
        return;
    }
    if [
        "xtask/src/nsis_harness.rs",
        "xtask/src/process.rs",
        "xtask/src/process/unix.rs",
        "xtask/src/release_v4.rs",
        "xtask/src/release_v4/recovery.rs",
        "xtask/src/release_v4/recovery/process_smoke.rs",
    ]
    .contains(&path)
    {
        classification.add_many(&[
            "rust_required",
            "desktop_native_required",
            "package_required",
            "release_required",
        ]);
        return;
    }
    if path == "xtask/src/updater_fixture.rs" {
        classification.add_many(&[
            "rust_required",
            "desktop_native_required",
            "package_required",
            "updater_required",
            "release_required",
        ]);
        return;
    }
    if path.starts_with("xtask/src/release_v4/") {
        unknown.push(path.to_owned());
        return;
    }
    if path.starts_with("xtask/") {
        if [
            "xtask/src/tauri_bundle.rs",
            "xtask/src/sbom.rs",
            "xtask/src/builtin_catalog.rs",
        ]
        .contains(&path)
        {
            classification.add_many(&["rust_required", "package_required"]);
        } else if path == "xtask/src/release_metadata.rs" {
            classification.add_many(&["updater_required", "release_required"]);
        } else if path == "xtask/src/supply_chain.rs" {
            classification.add("supply_chain_required");
        } else {
            classification.add("rust_required");
        }
        return;
    }
    if path.starts_with("tests/") {
        classification.add("rust_required");
        return;
    }
    if path.starts_with("songs/") || path.starts_with("builtin-songs/") {
        classification.add("package_required");
        return;
    }

    if path == "scripts/promote_v4_metadata.ps1" {
        classification.add_many(&["updater_required", "release_required"]);
        return;
    }
    if is_updater_script(path) {
        classification.add("updater_required");
        return;
    }
    if is_signing_script(path) {
        classification.add_many(&[
            "package_required",
            "release_required",
            "signing_proof_required",
        ]);
        return;
    }
    if is_nsis_boundary_script(path) {
        classification.add_many(&[
            "release_required",
            "package_required",
            "signing_proof_required",
        ]);
        return;
    }
    if is_release_script(path) {
        classification.add("release_required");
        return;
    }
    if path == "scripts/ci_classify.ps1" || path == "scripts/test_ci_classify.ps1" {
        unknown.push(path.to_owned());
        return;
    }
    if path.starts_with("scripts/")
        && (path.ends_with(".ps1")
            || path.ends_with(".psm1")
            || path.ends_with(".mjs")
            || path.starts_with("scripts/fixtures/")
            || path.starts_with("scripts/lib/"))
    {
        classification.add_many(&[
            "rust_required",
            "desktop_required",
            "desktop_native_required",
        ]);
        return;
    }

    unknown.push(path.to_owned());
}

fn is_windows_integration_frontend_path(path: &str) -> bool {
    [
        "apps/desktop/src/bridge/tauriBridge.ts",
        "apps/desktop/src/bridge/startupTelemetry.ts",
        "apps/desktop/src/platform/runtime.ts",
        "apps/desktop/src/platform/windowControls.ts",
        "apps/desktop/src/platform/startupWindowReveal.ts",
        "apps/desktop/src/platform/systemAppearance.ts",
        "apps/desktop/src/platform/externalLinks.ts",
        "apps/desktop/src/app/BootstrapGate.tsx",
        "apps/desktop/src/app/usePackagedSmokeTest.ts",
        "apps/desktop/src/main.tsx",
    ]
    .contains(&path)
        || path.starts_with("apps/desktop/src/bridge/generated/")
}

fn is_crate_manifest(path: &str) -> bool {
    let remainder = path.strip_prefix("crates/").unwrap_or_default();
    !remainder.contains('/') || remainder.ends_with("/Cargo.toml")
}

fn is_updater_script(path: &str) -> bool {
    let name = path.strip_prefix("scripts/").unwrap_or_default();
    name.ends_with(".ps1")
        && (name == "ci_tauri_update_e2e.ps1"
            || name == "ci_tauri_update_e2e_core.ps1"
            || name == "ci_build_updater_bridge.ps1"
            || name.starts_with("test_v4_updater_")
            || name.starts_with("verify_v4_updater_")
            || name.starts_with("updater_"))
}

fn is_signing_script(path: &str) -> bool {
    let name = path.strip_prefix("scripts/").unwrap_or_default();
    name.ends_with(".ps1")
        && [
            "sign_v4_authenticode.ps1",
            "verify_v4_authenticode.ps1",
            "v4_authenticode_crypto.ps1",
            "setup_v4_test_signing.ps1",
            "cleanup_v4_test_signing.ps1",
            "test_v4_authenticode.ps1",
            "test_v4_production_signing_contract.ps1",
        ]
        .contains(&name)
}

fn is_nsis_boundary_script(path: &str) -> bool {
    [
        "scripts/v4_nsis_smoke_boundary.ps1",
        "scripts/test_v4_nsis_smoke_boundary.ps1",
    ]
    .contains(&path)
}

fn is_release_script(path: &str) -> bool {
    let name = path.strip_prefix("scripts/").unwrap_or_default();
    name.ends_with(".ps1")
        && (name.starts_with("v4_release_")
            || name.starts_with("test_v4_release_")
            || [
                "orchestrate_v4_production_release.ps1",
                "verify_v4_release_runner.ps1",
                "cleanup_v4_release_state.ps1",
                "v4_release_draft_lookup.ps1",
                "test_v4_production_orchestrator.ps1",
            ]
            .contains(&name))
}

fn classify_normalized_paths(paths: Vec<String>, mode: DiffMode) -> Classification {
    if paths.is_empty() {
        return Classification::new("no changed paths");
    }
    if paths.iter().any(|path| path.chars().any(char::is_control)) {
        return Classification::full(format!(
            "malformed changed path requires full validation: {}",
            display_paths(&paths)
        ));
    }

    let mut classification = Classification::new(format!(
        "classified changed paths: {}",
        display_paths(&paths)
    ));
    let mut unknown = Vec::new();
    let mut static_only = Vec::new();
    for path in &paths {
        classify_path(
            path,
            &mut classification,
            &mut unknown,
            &mut static_only,
            mode,
        );
    }
    if !unknown.is_empty() {
        return Classification::full(format!(
            "unknown path requires full validation: {}",
            display_paths(&unknown)
        ));
    }
    let lanes = LANES
        .iter()
        .filter(|name| classification.lane(name))
        .copied()
        .collect::<Vec<_>>();
    classification.classification_reason = if !static_only.is_empty() && lanes.is_empty() {
        format!("static-only plus docs/site only: {}", display_paths(&paths))
    } else if !static_only.is_empty() {
        format!(
            "static-only plus required lanes ({}): {}",
            lanes.join(", "),
            display_paths(&paths)
        )
    } else if lanes.is_empty() {
        format!("docs/site only: {}", display_paths(&paths))
    } else {
        format!(
            "required lanes ({}): {}",
            lanes.join(", "),
            display_paths(&paths)
        )
    };
    classification
}

// Retain slash/dot/edge-whitespace normalization only for the explicit manual paths-file input.
pub(crate) fn classify_paths(raw_paths: impl IntoIterator<Item = String>) -> Classification {
    let paths = raw_paths
        .into_iter()
        .map(|path| normalize_manual_path(&path))
        .filter(|path| !path.trim().is_empty())
        .collect::<Vec<_>>();
    classify_normalized_paths(paths, DiffMode::PullRequest)
}

fn classify_git_paths(paths: Vec<String>, mode: DiffMode) -> Classification {
    if paths.iter().any(|path| {
        path.is_empty()
            || path.trim() != path
            || path.contains('\\')
            || path.chars().any(char::is_control)
    }) {
        return Classification::full(format!(
            "malformed changed path requires full validation: {}",
            display_paths(&paths)
        ));
    }
    classify_normalized_paths(paths, mode)
}

fn decode_git_paths(output: &[u8]) -> Result<Vec<String>> {
    output
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| {
            String::from_utf8(path.to_vec())
                .map_err(|_| "git changed path is not valid UTF-8; full validation required".into())
        })
        .collect()
}

fn git_output(root: &Path, args: &[&str]) -> Result<Output> {
    Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .map_err(|error| format!("could not run git {}: {error}", args.join(" ")).into())
}

fn ensure_commit_available(root: &Path, sha: &str) -> Result<()> {
    let sha = ci_artifacts::validate_commit_sha(sha, "base or head SHA")?;
    let expression = format!("{sha}^{{commit}}");
    let available = git_output(root, &["cat-file", "-e", expression.as_str()])?;
    if available.status.success() {
        return Ok(());
    }
    let fetch = git_output(
        root,
        &["fetch", "--no-tags", "--depth=1", "origin", sha.as_str()],
    )?;
    if !fetch.status.success() {
        return Err(format!("commit {sha} is unavailable and could not be fetched").into());
    }
    let available = git_output(root, &["cat-file", "-e", expression.as_str()])?;
    if available.status.success() {
        Ok(())
    } else {
        Err(format!("commit {sha} is unavailable and could not be fetched").into())
    }
}

fn source_head_matches(root: &Path, expected_head: &str) -> Result<()> {
    let output = git_output(root, &["rev-parse", "--verify", "HEAD^{commit}"])?;
    if !output.status.success() {
        return Err("checked-out source HEAD is unavailable".into());
    }
    let actual = String::from_utf8(output.stdout)
        .map_err(|_| "checked-out source HEAD is not valid UTF-8")?;
    if actual.trim() != expected_head {
        return Err(format!(
            "classifier head SHA {expected_head} does not match checked-out source HEAD {}",
            actual.trim()
        )
        .into());
    }
    Ok(())
}

fn merge_base(
    root: &Path,
    base: &str,
    head: &str,
    pull_request_refs: Option<&PullRequestRefs>,
) -> Result<String> {
    let output = git_output(root, &["merge-base", base, head])?;
    if output.status.success() {
        let value = String::from_utf8(output.stdout)
            .map_err(|_| "git merge-base returned invalid UTF-8")?;
        let value = value.trim();
        return ci_artifacts::validate_commit_sha(value, "merge-base SHA");
    }

    let shallow = git_output(root, &["rev-parse", "--is-shallow-repository"])?;
    if !shallow.status.success() {
        return Err("could not determine whether the checkout is shallow".into());
    }
    if String::from_utf8_lossy(&shallow.stdout).trim() != "true" {
        return Err("git merge-base could not find a common ancestor".into());
    }

    let refs = pull_request_refs
        .ok_or("bounded shallow PR fetch requires the base branch and pull request number")?;
    let (base_refspec, pr_head_refspec) = refs.fetch_refspecs(root)?;

    for deepen in SHALLOW_DEEPEN_STEPS {
        let depth = format!("--deepen={deepen}");
        let fetch = git_output(
            root,
            &[
                "-c",
                "credential.interactive=never",
                "-c",
                "http.lowSpeedLimit=1000",
                "-c",
                "http.lowSpeedTime=30",
                "fetch",
                "--no-tags",
                "--no-recurse-submodules",
                depth.as_str(),
                "origin",
                base_refspec.as_str(),
                pr_head_refspec.as_str(),
            ],
        )?;
        if !fetch.status.success() {
            continue;
        }
        let fetched_head = git_output(
            root,
            &["rev-parse", "--verify", "refs/sky-ci/pr-head^{commit}"],
        )?;
        if !fetched_head.status.success() {
            continue;
        }
        let fetched_head = String::from_utf8(fetched_head.stdout)
            .map_err(|_| "fetched pull request head is not valid UTF-8")?;
        if fetched_head.trim() != head {
            return Err(format!(
                "fetched pull request head {} does not match event head {head}",
                fetched_head.trim()
            )
            .into());
        }
        let output = git_output(root, &["merge-base", base, head])?;
        if output.status.success() {
            let value = String::from_utf8(output.stdout)
                .map_err(|_| "git merge-base returned invalid UTF-8")?;
            return ci_artifacts::validate_commit_sha(value.trim(), "merge-base SHA");
        }
    }
    Err(format!(
        "git merge-base unavailable after {} bounded shallow-history fetch attempts",
        SHALLOW_DEEPEN_STEPS.len()
    )
    .into())
}

fn changed_paths_from_git(
    root: &Path,
    mode: DiffMode,
    base: &str,
    head: &str,
    pull_request_refs: Option<&PullRequestRefs>,
) -> Result<Vec<String>> {
    let base = ci_artifacts::validate_commit_sha(base, "base or head SHA")?;
    let head = ci_artifacts::validate_commit_sha(head, "base or head SHA")?;
    source_head_matches(root, &head)?;
    let range_start = match mode {
        DiffMode::PullRequest => merge_base(root, &base, &head, pull_request_refs)?,
        DiffMode::Push => {
            ensure_commit_available(root, &base)?;
            base
        }
    };
    let output = git_output(
        root,
        &[
            "diff",
            "--no-renames",
            "--name-only",
            "-z",
            range_start.as_str(),
            head.as_str(),
        ],
    )?;
    if !output.status.success() {
        return Err("git diff failed for the requested base/head range".into());
    }
    decode_git_paths(&output.stdout)
}

fn classify_git_range_with_refs(
    root: &Path,
    mode: DiffMode,
    base: &str,
    head: &str,
    pull_request_refs: Option<&PullRequestRefs>,
) -> Classification {
    match changed_paths_from_git(root, mode, base, head, pull_request_refs) {
        Ok(paths) if paths.is_empty() => {
            Classification::full("empty changed-path diff requires full validation (fail-closed)")
        }
        Ok(paths) => classify_git_paths(paths, mode),
        Err(error) => Classification::full(format!("classifier failure (fail-closed): {error}")),
    }
}

fn classify_git_range(root: &Path, mode: DiffMode, base: &str, head: &str) -> Classification {
    classify_git_range_with_refs(root, mode, base, head, None)
}

fn classify_working_tree(root: &Path, base: Option<&str>) -> Result<Classification> {
    let mut command_specs: Vec<Vec<&str>> = Vec::new();
    let range;
    if let Some(base_ref) = base {
        if base_ref.starts_with('-') || base_ref.trim().is_empty() {
            return Ok(Classification::full(format!(
                "invalid option-looking or empty base ref: {base_ref} (fail-closed)"
            )));
        }
        let valid = git_output(
            root,
            &[
                "rev-parse",
                "--verify",
                "--end-of-options",
                format!("{base_ref}^{{commit}}").as_str(),
            ],
        )?;
        if !valid.status.success() {
            return Ok(Classification::full(format!(
                "invalid or missing base ref: {base_ref} (fail-closed)"
            )));
        }
        let base_commit = String::from_utf8_lossy(&valid.stdout).trim().to_owned();
        if base_commit.is_empty() {
            return Ok(Classification::full("empty base commit SHA (fail-closed)"));
        }
        let mb = git_output(root, &["merge-base", "--", base_commit.as_str(), "HEAD"])?;
        if !mb.status.success() {
            return Ok(Classification::full(format!(
                "could not resolve merge-base for {base_ref} and HEAD (fail-closed)"
            )));
        }
        let merge_base = String::from_utf8_lossy(&mb.stdout).trim().to_owned();
        if merge_base.is_empty() {
            return Ok(Classification::full("empty merge-base (fail-closed)"));
        }
        range = format!("{merge_base}..HEAD");
        command_specs.push(vec![
            "diff",
            "--name-only",
            "--no-renames",
            "-z",
            range.as_str(),
            "--",
        ]);
    }
    command_specs.push(vec![
        "diff",
        "--cached",
        "--name-only",
        "--no-renames",
        "-z",
        "HEAD",
        "--",
    ]);
    command_specs.push(vec!["diff", "--name-only", "--no-renames", "-z", "--"]);
    command_specs.push(vec!["ls-files", "--others", "--exclude-standard", "-z"]);

    let mut paths = Vec::new();
    for args in &command_specs {
        let output = git_output(root, args.as_slice())?;
        if !output.status.success() {
            return Err(format!("git {} failed", args.join(" ")).into());
        }
        paths.extend(decode_git_paths(&output.stdout)?);
    }
    paths.sort();
    paths.dedup();
    Ok(classify_git_paths(paths, DiffMode::PullRequest))
}

fn parse_arguments(args: &[String]) -> Result<Classification> {
    let working_tree_count = args.iter().filter(|arg| *arg == "--working-tree").count();
    if working_tree_count > 1 {
        return Err("duplicate --working-tree option specified".into());
    }
    if working_tree_count == 1 {
        if args.iter().any(|arg| arg == "--github-output") {
            return Err("--github-output cannot be combined with --working-tree".into());
        }
        let mut base = None;
        let mut index = 0;
        while index < args.len() {
            match args[index].as_str() {
                "--working-tree" => {}
                "--base" | "--base-sha" => {
                    if base.is_some() {
                        return Err("duplicate --base option specified".into());
                    }
                    index += 1;
                    let val = args.get(index).ok_or("missing value for --base")?;
                    if val.starts_with("--") {
                        return Err(format!("invalid value for --base: {val}").into());
                    }
                    base = Some(val.clone());
                }
                opt => {
                    return Err(
                        format!("option {opt} cannot be combined with --working-tree").into(),
                    );
                }
            }
            index += 1;
        }
        return classify_working_tree(&crate::repo::root(), base.as_deref());
    }
    let mut full = false;
    let mut base = None;
    let mut head = None;
    let mut diff_mode = None;
    let mut pr_number = None;
    let mut base_ref = None;
    let mut paths_file = None;
    let mut index = 0;
    while index < args.len() {
        let option = args[index].as_str();
        match option {
            "--full" => full = true,
            "--base-sha" | "--base" => {
                index += 1;
                base = Some(
                    args.get(index)
                        .ok_or("missing value for --base-sha")?
                        .clone(),
                );
            }
            "--head-sha" | "--head" => {
                index += 1;
                head = Some(
                    args.get(index)
                        .ok_or("missing value for --head-sha")?
                        .clone(),
                );
            }
            "--diff-mode" => {
                index += 1;
                diff_mode = Some(DiffMode::parse(
                    args.get(index).ok_or("missing value for --diff-mode")?,
                )?);
            }
            "--pr-number" => {
                index += 1;
                pr_number = Some(
                    args.get(index)
                        .ok_or("missing value for --pr-number")?
                        .clone(),
                );
            }
            "--base-ref" => {
                index += 1;
                base_ref = Some(
                    args.get(index)
                        .ok_or("missing value for --base-ref")?
                        .clone(),
                );
            }
            "--paths-file" => {
                index += 1;
                paths_file = Some(
                    args.get(index)
                        .ok_or("missing value for --paths-file")?
                        .clone(),
                );
            }
            "--github-output" => {
                index += 1;
                let _ = args.get(index).ok_or("missing value for --github-output")?;
            }
            _ => return Err(format!("unknown classifier option: {option}").into()),
        }
        index += 1;
    }

    if full {
        return Ok(Classification::full("full validation requested"));
    }
    if paths_file.is_some()
        && (base.is_some()
            || head.is_some()
            || diff_mode.is_some()
            || pr_number.is_some()
            || base_ref.is_some())
    {
        return Err(
            "paths file cannot be combined with diff-mode, PR refs, or base/head SHAs".into(),
        );
    }
    if let Some(path) = paths_file {
        let source = fs::read_to_string(path).map_err(|_| "paths file does not exist")?;
        let raw_paths = source
            .strip_prefix('\u{feff}')
            .unwrap_or(&source)
            .lines()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        Ok(classify_paths(raw_paths))
    } else {
        match (base, head) {
            (Some(base), Some(head)) => match diff_mode {
                Some(DiffMode::PullRequest) => {
                    let (Some(number), Some(base_ref)) = (pr_number, base_ref) else {
                        return Err(
                            "--pr-number and --base-ref are required in PR diff mode".into()
                        );
                    };
                    let refs = PullRequestRefs::parse(&number, base_ref)?;
                    Ok(classify_git_range_with_refs(
                        &crate::repo::root(),
                        DiffMode::PullRequest,
                        &base,
                        &head,
                        Some(&refs),
                    ))
                }
                Some(DiffMode::Push) => {
                    if pr_number.is_some() || base_ref.is_some() {
                        return Err("PR refs cannot be combined with push diff mode".into());
                    }
                    Ok(classify_git_range(
                        &crate::repo::root(),
                        DiffMode::Push,
                        &base,
                        &head,
                    ))
                }
                None => Err("--diff-mode pr|push is required with base/head SHAs".into()),
            },
            (None, None) => Err("base and head SHAs are required".into()),
            _ => Err("base and head SHAs must be supplied together".into()),
        }
    }
}

fn github_delimiter(value: &str) -> String {
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let mut sequence = 0_u64;
    loop {
        let delimiter = format!(
            "SKY_XTASK_OUTPUT_{}_{}_{}",
            std::process::id(),
            time,
            sequence
        );
        if !value.lines().any(|line| line == delimiter) {
            return delimiter;
        }
        sequence += 1;
    }
}

pub(crate) fn append_github_output(path: &Path, classification: &Classification) -> Result<()> {
    let mut output = OpenOptions::new().create(true).append(true).open(path)?;
    for (index, lane) in LANES.iter().enumerate() {
        writeln!(output, "{lane}={}", classification.lanes[index])?;
    }
    let delimiter = github_delimiter(&classification.classification_reason);
    writeln!(output, "classification_reason<<{delimiter}")?;
    writeln!(output, "{}", classification.classification_reason)?;
    writeln!(output, "{delimiter}")?;
    output.flush()?;
    Ok(())
}

pub(crate) fn run(args: &[String]) -> Result<()> {
    let is_working_tree = args.iter().any(|arg| arg == "--working-tree");
    if is_working_tree {
        // Local working tree preview: purely read-only, never write to --github-output.
        let classification = match parse_arguments(args) {
            Ok(classification) => classification,
            Err(error) => {
                Classification::full(format!("classifier failure (fail-closed): {error}"))
            }
        };
        println!("{}", classification.render());
        return Ok(());
    }

    let (classification, github_output) = match parse_arguments(args) {
        Ok(classification) => (
            classification,
            args.iter()
                .position(|arg| arg == "--github-output")
                .and_then(|index| args.get(index + 1).cloned()),
        ),
        Err(error) => (
            Classification::full(format!("classifier failure (fail-closed): {}", error)),
            args.iter()
                .position(|arg| arg == "--github-output")
                .and_then(|index| args.get(index + 1).cloned()),
        ),
    };
    if let Some(path) = github_output {
        append_github_output(Path::new(&path), &classification)?;
    }
    println!("{}", classification.render());
    Ok(())
}
#[cfg(test)]
#[path = "ci_classifier/tests.rs"]
mod tests;
