#![forbid(unsafe_code)]

use crate::{Result, ci_artifacts};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) const LANES: [&str; 11] = [
    "rust_required",
    "desktop_required",
    "desktop_native_required",
    "desktop_e2e_required",
    "package_required",
    "updater_required",
    "release_required",
    "signing_proof_required",
    "malware_proof_required",
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
        classification.add_many(&[
            "release_required",
            "signing_proof_required",
            "malware_proof_required",
        ]);
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

    if path == "rust/xtask/src/release_v4/malware.rs" {
        classification.add_many(&[
            "rust_required",
            "release_required",
            "malware_proof_required",
        ]);
        return;
    }
    if path == "scripts/scan_v4_malware_exact.py"
        || path.starts_with("scripts/test_v4_malware_")
        || path == "scripts/ci_bootstrap_clamav.sh"
        || path == "scripts/test_ci_bootstrap_clamav.sh"
    {
        classification.add_many(&["release_required", "malware_proof_required"]);
        return;
    }
    if [
        "rust/xtask/src/release_signing.rs",
        "rust/xtask/src/authenticode.rs",
        "rust/xtask/src/updater_trust.rs",
    ]
    .contains(&path)
        || path.starts_with("rust/xtask/src/release_signing/")
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
    if path.starts_with("rust/xtask/src/checks/") {
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
            "malware_proof_required",
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
            "malware_proof_required",
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

    if path == "Cargo.toml" {
        classification.add_many(&[
            "rust_required",
            "package_required",
            "updater_required",
            "release_required",
            "supply_chain_required",
        ]);
        return;
    }
    if ["rust/Cargo.toml", "rust/Cargo.lock"].contains(&path) {
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
        "rust/crates/sky_windows_security/src/process_tree.rs",
        "rust/crates/sky_windows_security/src/registry.rs",
        "rust/crates/sky_windows_security/src/win32.rs",
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
    if path == "rust/rust-toolchain.toml" || path.starts_with(".cargo/") {
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
    if path.starts_with("rust/crates/") {
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
        "rust/xtask/src/ci_classifier.rs",
        "rust/xtask/src/ci.rs",
        "rust/xtask/src/main.rs",
    ]
    .contains(&path)
    {
        unknown.push(path.to_owned());
        return;
    }
    if [
        "rust/xtask/src/ci_artifacts.rs",
        "rust/xtask/src/ci_candidate.rs",
        "rust/xtask/src/ci_bridge.rs",
        "rust/xtask/src/calibration.rs",
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
        "rust/xtask/src/nsis_harness.rs",
        "rust/xtask/src/process.rs",
        "rust/xtask/src/process/unix.rs",
        "rust/xtask/src/release_v4.rs",
        "rust/xtask/src/release_v4/recovery.rs",
        "rust/xtask/src/release_v4/recovery/process_smoke.rs",
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
    if path == "rust/xtask/src/updater_fixture.rs" {
        classification.add_many(&[
            "rust_required",
            "desktop_native_required",
            "package_required",
            "updater_required",
            "release_required",
        ]);
        return;
    }
    if path.starts_with("rust/xtask/") {
        if [
            "rust/xtask/src/tauri_bundle.rs",
            "rust/xtask/src/sbom.rs",
            "rust/xtask/src/builtin_catalog.rs",
        ]
        .contains(&path)
        {
            classification.add_many(&["rust_required", "package_required"]);
        } else if path == "rust/xtask/src/release_metadata.rs" {
            classification.add_many(&["updater_required", "release_required"]);
        } else if path == "rust/xtask/src/supply_chain.rs" {
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
            "malware_proof_required",
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
    if path.starts_with("scripts/") {
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
    let remainder = path.strip_prefix("rust/crates/").unwrap_or_default();
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

fn parse_arguments(args: &[String]) -> Result<Classification> {
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
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn is_tauri_module_specifier(statement: &str) -> bool {
        statement.contains("@tauri-apps/api/") || statement.contains("@tauri-apps/plugin-")
    }

    fn is_type_only_import_statement(statement: &str) -> bool {
        let statement = statement.trim_start();
        if statement.starts_with("import type ") || statement.starts_with("export type ") {
            return true;
        }

        let Some(open) = statement.find('{') else {
            return false;
        };
        if statement.starts_with("import ") && !statement["import ".len()..open].trim().is_empty() {
            return false;
        }
        let Some(close) = statement[open + 1..].find('}') else {
            return false;
        };
        let specifiers = &statement[open + 1..open + 1 + close];
        let specifiers = specifiers
            .split(',')
            .map(str::trim)
            .filter(|specifier| !specifier.is_empty())
            .collect::<Vec<_>>();
        !specifiers.is_empty()
            && specifiers
                .iter()
                .all(|specifier| specifier.starts_with("type "))
    }

    fn contains_tauri_runtime_import(source: &str) -> bool {
        let mut declaration = None::<String>;
        for raw_line in source.lines() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with("//") {
                continue;
            }

            if let Some(statement) = declaration.as_mut() {
                statement.push(' ');
                statement.push_str(line);
            } else if line.starts_with("import ") || line.starts_with("export ") {
                declaration = Some(line.to_owned());
            } else {
                let is_dynamic_import = line.contains("import(") || line.contains("import (");
                let is_type_query = line.starts_with("type ") || line.starts_with("export type ");
                if is_dynamic_import && !is_type_query && is_tauri_module_specifier(line) {
                    return true;
                }
                continue;
            }

            let statement = declaration.as_deref().unwrap_or_default();
            if !is_type_only_import_statement(statement) && is_tauri_module_specifier(statement) {
                return true;
            }
            if statement.contains(';') || statement.contains(" from ") {
                declaration = None;
            }
        }
        false
    }

    fn temp_dir(name: &str) -> PathBuf {
        let path = ci_artifacts::unique_sibling(&std::env::temp_dir().join(name), "test");
        fs::create_dir(&path).unwrap();
        path
    }

    fn assert_lanes(classification: &Classification, expected: &[&str]) {
        for lane in LANES {
            assert_eq!(
                classification.lane(lane),
                expected.contains(&lane),
                "{lane}"
            );
        }
    }

    #[test]
    fn preserves_exact_lane_output_shape_and_reason_golden() {
        let classification = classify_paths(["apps/desktop/src/App.tsx".to_owned()]);
        assert_eq!(
            classification.render(),
            "rust_required=false\ndesktop_required=true\ndesktop_native_required=false\ndesktop_e2e_required=true\npackage_required=false\nupdater_required=false\nrelease_required=false\nsigning_proof_required=false\nmalware_proof_required=false\nsupply_chain_required=false\nsite_required=false\nclassification_reason=required lanes (desktop_required, desktop_e2e_required): apps/desktop/src/App.tsx"
        );
    }

    #[test]
    fn matches_positive_and_union_classifier_matrix() {
        for path in ["README.md", "CHANGELOG.md", "docs/architecture.md"] {
            assert_lanes(&classify_paths([path.to_owned()]), &[]);
        }
        assert_lanes(&classify_paths(["docs/releases/v4.0.2.md".to_owned()]), &[]);
        assert_lanes(
            &classify_paths(["apps/website/package.json".to_owned()]),
            &["site_required"],
        );
        assert_lanes(
            &classify_paths(["apps/website/src/pages/index.astro".to_owned()]),
            &["site_required"],
        );
        assert_lanes(
            &classify_paths([
                "apps/desktop/src/App.tsx".to_owned(),
                "apps/website/src/pages/index.astro".to_owned(),
            ]),
            &["desktop_required", "desktop_e2e_required", "site_required"],
        );
        let static_site = classify_paths([
            ".config/rust_architecture_allowlist.json".to_owned(),
            "apps/website/src/pages/index.astro".to_owned(),
        ]);
        assert_lanes(&static_site, &["site_required"]);
        assert!(
            static_site
                .classification_reason
                .starts_with("static-only plus required lanes")
        );

        let desktop = classify_paths([
            "docs/releases/v4.0.2.md".to_owned(),
            "apps/desktop/src/App.tsx".to_owned(),
        ]);
        assert_lanes(&desktop, &["desktop_required", "desktop_e2e_required"]);
        assert!(
            desktop
                .classification_reason
                .starts_with("static-only plus required lanes")
        );

        let lanes = classify_paths([
            "apps/desktop/src-tauri/tauri.conf.json".to_owned(),
            "rust/xtask/src/ci_candidate.rs".to_owned(),
            "scripts/ci_build_updater_bridge.ps1".to_owned(),
        ]);
        assert_lanes(
            &lanes,
            &[
                "rust_required",
                "desktop_required",
                "desktop_native_required",
                "package_required",
                "updater_required",
                "signing_proof_required",
                "malware_proof_required",
            ],
        );
        assert_lanes(
            &classify_paths(["scripts/ci_build_updater_bridge.ps1".to_owned()]),
            &["updater_required"],
        );

        let nsis_owner = classify_paths(["rust/xtask/src/nsis_harness.rs".to_owned()]);
        assert_lanes(
            &nsis_owner,
            &[
                "rust_required",
                "desktop_native_required",
                "package_required",
                "release_required",
            ],
        );
        for path in [
            "rust/xtask/src/process.rs",
            "rust/xtask/src/process/unix.rs",
            "rust/xtask/src/release_v4.rs",
            "rust/xtask/src/release_v4/recovery.rs",
            "rust/xtask/src/release_v4/recovery/process_smoke.rs",
        ] {
            assert_lanes(
                &classify_paths([path.to_owned()]),
                &[
                    "rust_required",
                    "desktop_native_required",
                    "package_required",
                    "release_required",
                ],
            );
        }
        let updater_owner = classify_paths(["rust/xtask/src/updater_fixture.rs".to_owned()]);
        assert_lanes(
            &updater_owner,
            &[
                "rust_required",
                "desktop_native_required",
                "package_required",
                "updater_required",
                "release_required",
            ],
        );
        let registry_owner =
            classify_paths(["rust/crates/sky_windows_security/src/registry.rs".to_owned()]);
        assert_lanes(
            &registry_owner,
            &[
                "rust_required",
                "desktop_native_required",
                "package_required",
                "release_required",
            ],
        );
    }

    #[test]
    fn preserves_current_positive_path_matrix_and_script_categories() {
        let cases: &[(&str, &[&str])] = &[
            ("README.md", &[]),
            ("docs/releases/v4.0.2.md", &[]),
            ("apps/website/package.json", &["site_required"]),
            (".github/workflows/pages.yml", &["site_required"]),
            (".bun-version", &LANES),
            (
                ".github/workflows/release-v4.yml",
                &[
                    "release_required",
                    "signing_proof_required",
                    "malware_proof_required",
                ],
            ),
            (".github/workflows/ci.yml", &LANES),
            (
                "apps/desktop/src/App.tsx",
                &["desktop_required", "desktop_e2e_required"],
            ),
            (
                "apps/desktop/tests/e2e/library.spec.ts",
                &["desktop_required", "desktop_e2e_required"],
            ),
            (
                "apps/desktop/package.json",
                &[
                    "desktop_required",
                    "desktop_e2e_required",
                    "package_required",
                    "updater_required",
                    "supply_chain_required",
                ],
            ),
            (
                "apps/desktop/src-tauri/src/commands.rs",
                &[
                    "rust_required",
                    "desktop_required",
                    "desktop_native_required",
                    "package_required",
                ],
            ),
            (
                "apps/desktop/src-tauri/src/native_update.rs",
                &[
                    "rust_required",
                    "desktop_required",
                    "desktop_native_required",
                    "package_required",
                    "updater_required",
                ],
            ),
            (
                "apps/desktop/src-tauri/src/main.rs",
                &[
                    "rust_required",
                    "desktop_required",
                    "desktop_native_required",
                    "package_required",
                ],
            ),
            (
                "apps/desktop/src-tauri/tauri.conf.json",
                &[
                    "desktop_required",
                    "desktop_native_required",
                    "package_required",
                    "updater_required",
                    "signing_proof_required",
                    "malware_proof_required",
                ],
            ),
            (
                "apps/desktop/src-tauri/nsis-hooks.nsh",
                &[
                    "desktop_native_required",
                    "package_required",
                    "signing_proof_required",
                    "malware_proof_required",
                ],
            ),
            (
                "apps/desktop/src-tauri/capabilities/main.json",
                &[
                    "desktop_required",
                    "desktop_native_required",
                    "package_required",
                ],
            ),
            (
                "apps/desktop/src-tauri/icons/icon.ico",
                &["package_required"],
            ),
            (
                "rust/Cargo.toml",
                &[
                    "rust_required",
                    "package_required",
                    "updater_required",
                    "release_required",
                    "supply_chain_required",
                ],
            ),
            (
                "rust/Cargo.lock",
                &[
                    "rust_required",
                    "package_required",
                    "updater_required",
                    "release_required",
                    "supply_chain_required",
                ],
            ),
            (
                "rust/crates/sky_player/Cargo.toml",
                &[
                    "rust_required",
                    "desktop_native_required",
                    "package_required",
                    "supply_chain_required",
                ],
            ),
            (
                "rust/crates/sky_player/src/lib.rs",
                &[
                    "rust_required",
                    "desktop_native_required",
                    "package_required",
                ],
            ),
            (
                "rust/xtask/src/ci_candidate.rs",
                &[
                    "rust_required",
                    "desktop_native_required",
                    "package_required",
                    "updater_required",
                ],
            ),
            (
                "rust/xtask/src/release_signing.rs",
                &[
                    "rust_required",
                    "package_required",
                    "updater_required",
                    "release_required",
                    "signing_proof_required",
                ],
            ),
            (
                "rust/xtask/src/release_v4/malware.rs",
                &[
                    "rust_required",
                    "release_required",
                    "malware_proof_required",
                ],
            ),
            ("rust/xtask/src/checks/hosted_signing.rs", &LANES),
            ("scripts/ci_build_updater_bridge.ps1", &["updater_required"]),
            (
                "scripts/sign_v4_authenticode.ps1",
                &[
                    "package_required",
                    "release_required",
                    "signing_proof_required",
                ],
            ),
            (
                "scripts/test_v4_authenticode.ps1",
                &[
                    "package_required",
                    "release_required",
                    "signing_proof_required",
                ],
            ),
            (
                "scripts/v4_nsis_smoke_boundary.ps1",
                &[
                    "release_required",
                    "package_required",
                    "signing_proof_required",
                    "malware_proof_required",
                ],
            ),
            (
                "scripts/scan_v4_malware_exact.py",
                &["release_required", "malware_proof_required"],
            ),
            (
                "scripts/ci_bootstrap_clamav.sh",
                &["release_required", "malware_proof_required"],
            ),
            ("scripts/v4_release_pipeline.ps1", &["release_required"]),
            (
                "scripts/promote_v4_metadata.ps1",
                &["updater_required", "release_required"],
            ),
        ];
        for (path, expected) in cases {
            let classification = classify_paths([(*path).to_owned()]);
            for lane in LANES {
                assert_eq!(
                    classification.lane(lane),
                    expected.contains(&lane),
                    "{path}: {lane}"
                );
            }
        }

        let main_ui =
            classify_normalized_paths(vec!["apps/desktop/src/App.tsx".to_owned()], DiffMode::Push);
        assert_lanes(
            &main_ui,
            &[
                "desktop_required",
                "desktop_e2e_required",
                "package_required",
            ],
        );
        let main_site =
            classify_normalized_paths(vec!["apps/website/package.json".to_owned()], DiffMode::Push);
        assert_lanes(&main_site, &["site_required"]);
    }
    #[test]
    fn rust_toolchain_resolver_and_setup_action_select_all_validation_lanes() {
        for path in [
            "scripts/toolchain-pin.mjs",
            ".github/actions/setup-rust-pinned/action.yml",
        ] {
            assert_lanes(&classify_paths([path.to_owned()]), &LANES);
        }
        assert_lanes(
            &classify_paths(["rust/rust-toolchain.toml".to_owned()]),
            &[
                "rust_required",
                "desktop_required",
                "desktop_native_required",
                "package_required",
                "updater_required",
                "release_required",
            ],
        );
        assert_lanes(
            &classify_paths(["apps/website/src/pages/index.astro".to_owned()]),
            &["site_required"],
        );
    }

    #[test]
    fn windows_tauri_integration_frontend_paths_select_package_qualification() {
        for path in [
            "apps/desktop/src/bridge/tauriBridge.ts",
            "apps/desktop/src/bridge/startupTelemetry.ts",
            "apps/desktop/src/bridge/generated/commands.ts",
            "apps/desktop/src/bridge/generated/nested/Example.ts",
            "apps/desktop/src/platform/runtime.ts",
            "apps/desktop/src/platform/windowControls.ts",
            "apps/desktop/src/platform/startupWindowReveal.ts",
            "apps/desktop/src/platform/systemAppearance.ts",
            "apps/desktop/src/platform/externalLinks.ts",
            "apps/desktop/src/app/BootstrapGate.tsx",
            "apps/desktop/src/app/usePackagedSmokeTest.ts",
            "apps/desktop/src/main.tsx",
        ] {
            assert_lanes(
                &classify_paths([path.to_owned()]),
                &[
                    "desktop_required",
                    "desktop_native_required",
                    "desktop_e2e_required",
                    "package_required",
                ],
            );
        }

        for path in [
            "apps/desktop/src/components/PlaybackControls.tsx",
            "apps/desktop/src/styles/app.css",
            "apps/desktop/src/app/App.tsx",
            "apps/desktop/src/bridge/generatedish/Example.ts",
        ] {
            assert_lanes(
                &classify_paths([path.to_owned()]),
                &["desktop_required", "desktop_e2e_required"],
            );
        }
    }

    #[test]
    fn production_tauri_runtime_imports_are_windows_package_qualified() {
        assert!(contains_tauri_runtime_import(
            "import { invoke } from '@tauri-apps/api/core';"
        ));
        assert!(contains_tauri_runtime_import(
            r"import {
  openUrl,
} from '@tauri-apps/plugin-opener';"
        ));
        assert!(contains_tauri_runtime_import(
            "const api = await import('@tauri-apps/api/window');"
        ));
        assert!(!contains_tauri_runtime_import(
            "import type { Window } from '@tauri-apps/api/window';"
        ));
        assert!(!contains_tauri_runtime_import(
            "import { type Window } from '@tauri-apps/api/window';"
        ));
        assert!(contains_tauri_runtime_import(
            "import WindowApi, { type Window } from '@tauri-apps/api/window';"
        ));
        assert!(!contains_tauri_runtime_import(
            "type WindowApi = import('@tauri-apps/api/window').Window;"
        ));

        let repository_root = crate::repo::root();
        let source_root = repository_root.join("apps/desktop/src");
        let mut observed = Vec::new();
        for entry in walkdir::WalkDir::new(&source_root).follow_links(false) {
            let entry = entry.unwrap();
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            let extension = path.extension().and_then(|extension| extension.to_str());
            if !matches!(extension, Some("ts" | "tsx")) {
                continue;
            }
            let file_name = path
                .file_name()
                .and_then(|file_name| file_name.to_str())
                .unwrap_or_default();
            if file_name.ends_with(".d.ts")
                || file_name.contains(".test.")
                || file_name.contains(".spec.")
            {
                continue;
            }

            let source = fs::read_to_string(path).unwrap();
            if !contains_tauri_runtime_import(&source) {
                continue;
            }
            let relative = path
                .strip_prefix(&repository_root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            let classification = classify_paths([relative.clone()]);
            assert!(
                classification.lane("package_required"),
                "direct Tauri runtime import lacks Windows package qualification: {relative}"
            );
            assert!(
                classification.lane("desktop_native_required"),
                "direct Tauri runtime import lacks Windows native validation: {relative}"
            );
            observed.push(relative);
        }

        for expected in [
            "apps/desktop/src/bridge/startupTelemetry.ts",
            "apps/desktop/src/bridge/tauriBridge.ts",
            "apps/desktop/src/platform/externalLinks.ts",
            "apps/desktop/src/platform/runtime.ts",
            "apps/desktop/src/platform/startupWindowReveal.ts",
            "apps/desktop/src/platform/systemAppearance.ts",
            "apps/desktop/src/platform/windowControls.ts",
        ] {
            assert!(
                observed.iter().any(|path| path == expected),
                "expected production Tauri runtime import was not scanned: {expected}"
            );
        }
    }

    #[test]
    fn preserves_legacy_reason_and_mixed_path_regressions() {
        assert!(
            classify_paths(["docs/releases/v4.0.2.md".to_owned()])
                .classification_reason
                .starts_with("static-only")
        );

        for path in [
            "scripts/v4_release_pipeline.ps1",
            ".github/workflows/release-v4.yml",
        ] {
            let classification =
                classify_paths(["docs/releases/v4.0.2.md".to_owned(), path.to_owned()]);
            let expected = if path == ".github/workflows/release-v4.yml" {
                &[
                    "release_required",
                    "signing_proof_required",
                    "malware_proof_required",
                ][..]
            } else {
                &["release_required"][..]
            };
            assert_lanes(&classification, expected);
        }

        let no_changes = classify_paths(std::iter::empty());
        assert_lanes(&no_changes, &[]);
        assert_eq!(no_changes.classification_reason, "no changed paths");
    }

    #[test]
    fn normalizes_slashes_whitespace_and_repeated_dot_prefixes() {
        assert_lanes(&classify_paths(["  .\\./README.md  ".to_owned()]), &[]);
        assert_lanes(
            &classify_paths([".\\apps\\desktop\\src\\App.tsx".to_owned()]),
            &["desktop_required", "desktop_e2e_required"],
        );
    }

    #[test]
    fn git_paths_are_exact_and_malformed_names_fail_closed() {
        for path in [
            "README.md ",
            "README.md\n",
            "apps\\desktop\\src\\App.tsx",
            " README.md",
        ] {
            let classification = classify_git_paths(vec![path.to_owned()], DiffMode::PullRequest);
            assert_lanes(&classification, &LANES);
            assert!(
                classification
                    .classification_reason
                    .starts_with("malformed changed path")
            );
        }

        assert_lanes(
            &classify_git_paths(
                vec!["apps/desktop/src/App.tsx".to_owned()],
                DiffMode::PullRequest,
            ),
            &["desktop_required", "desktop_e2e_required"],
        );
        // Manual paths-file input deliberately retains its historical normalization contract.
        assert_lanes(
            &classify_paths([" .\\apps\\desktop\\src\\App.tsx ".to_owned()]),
            &["desktop_required", "desktop_e2e_required"],
        );
    }

    #[test]
    fn git_path_decoding_preserves_trailing_space_and_rejects_invalid_utf8() {
        let paths = decode_git_paths(b"README.md \0apps/desktop/src/App.tsx\0").unwrap();
        assert_eq!(paths, ["README.md ", "apps/desktop/src/App.tsx"]);
        assert!(decode_git_paths(b"README.md\xff\0").is_err());
    }

    #[test]
    fn unknown_or_malformed_paths_fail_closed_and_workflow_dispatch_is_full() {
        for path in [
            "unclassified/file.txt",
            "desktop/src/App.tsx",
            "desktop/src-tauri/Cargo.toml",
            "site/package.json",
            "site/src/pages/index.astro",
            ".github/workflows/unknown.yml",
            ".github/actions/unknown/action.yml",
            "rust/xtask/src/ci_classifier.rs",
            "rust/xtask/src/ci.rs",
            "rust/xtask/src/main.rs",
            "scripts/ci_classify.ps1",
            "scripts/test_ci_classify.ps1",
        ] {
            let classification = classify_paths([path.to_owned()]);
            assert_lanes(&classification, &LANES);
            assert!(
                classification
                    .classification_reason
                    .starts_with("unknown path")
            );
        }
        let malformed = classify_paths(["apps/desktop/src/a\nfile.tsx".to_owned()]);
        assert_lanes(&malformed, &LANES);
        assert!(
            malformed
                .classification_reason
                .starts_with("malformed changed path")
        );
        let full = Classification::full("full validation requested");
        assert_lanes(&full, &LANES);
        assert_eq!(full.classification_reason, "full validation requested");
        let dispatch = parse_arguments(&["--full".to_owned()]).unwrap();
        assert_lanes(&dispatch, &LANES);
        assert_eq!(dispatch.classification_reason, "full validation requested");
    }

    #[test]
    fn invalid_base_head_arguments_fail_closed_and_write_all_github_outputs() {
        let root = temp_dir("sky-xtask-classifier-invalid-sha");
        let output = root.join("github-output.txt");
        let args = vec![
            "--base-sha".to_owned(),
            "0000000000000000000000000000000000000000".to_owned(),
            "--head-sha".to_owned(),
            "1234567890abcdef1234567890abcdef12345678".to_owned(),
            "--diff-mode".to_owned(),
            "pr".to_owned(),
            "--github-output".to_owned(),
            output.to_string_lossy().into_owned(),
        ];
        run(&args).unwrap();
        let contents = fs::read_to_string(&output).unwrap();
        for lane in LANES {
            assert!(contents.contains(&format!("{lane}=true\n")), "{lane}");
        }
        assert!(contents.contains("classification_reason<<"));
        assert!(contents.contains("classifier failure (fail-closed)"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn github_output_uses_multiline_safe_encoding_for_reason() {
        let root = temp_dir("sky-xtask-github-output-test");
        let path = root.join("output.txt");
        let mut classification = Classification::new("first line\nsecond=value");
        classification.add("rust_required");
        append_github_output(&path, &classification).unwrap();
        let output = fs::read_to_string(&path).unwrap();
        assert!(output.starts_with("rust_required=true\ndesktop_required=false\n"));
        assert!(output.contains("classification_reason<<SKY_XTASK_OUTPUT_"));
        assert!(output.contains("\nfirst line\nsecond=value\nSKY_XTASK_OUTPUT_"));
        fs::remove_dir_all(root).unwrap();
    }

    fn git(root: &Path, args: &[&str]) -> Output {
        let output = Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }

    fn git_sha(root: &Path) -> String {
        String::from_utf8(git(root, &["rev-parse", "HEAD"]).stdout)
            .unwrap()
            .trim()
            .to_owned()
    }

    fn file_url(path: &Path) -> String {
        let canonical = path.canonicalize().unwrap();
        let value = canonical.to_string_lossy();
        let value = value
            .strip_prefix("\\\\?\\")
            .unwrap_or(&value)
            .replace('\\', "/");
        if cfg!(windows) {
            format!("file:///{value}")
        } else {
            format!("file://{value}")
        }
    }

    #[test]
    fn pr_uses_merge_base_when_base_advances_and_diff_keeps_both_rename_paths() {
        let root = temp_dir("sky-xtask-classifier-merge-base");
        let source = root.join("source");
        let remote = root.join("origin.git");
        let shallow = root.join("shallow");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&remote).unwrap();
        git(&remote, &["init", "--bare", "--quiet"]);
        git(&source, &["init", "--quiet"]);
        git(&source, &["config", "user.name", "Classifier Test"]);
        git(
            &source,
            &["config", "user.email", "classifier@example.invalid"],
        );
        fs::create_dir_all(source.join("apps/desktop/src")).unwrap();
        fs::write(source.join("README.md"), "base\n").unwrap();
        fs::write(
            source.join("apps/desktop/src/Old.tsx"),
            "export const Old = 1;\n",
        )
        .unwrap();
        git(&source, &["add", "--all"]);
        git(&source, &["commit", "--quiet", "-m", "common base"]);
        let merge_base = git_sha(&source);
        git(&source, &["branch", "feature"]);

        fs::write(source.join("README.md"), "main advanced\n").unwrap();
        fs::create_dir_all(source.join("apps/website")).unwrap();
        fs::write(source.join("apps/website/package.json"), "{}\n").unwrap();
        git(&source, &["add", "--all"]);
        git(&source, &["commit", "--quiet", "-m", "advance main"]);
        let base = git_sha(&source);
        git(
            &source,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
        git(
            &source,
            &["push", "--quiet", "origin", "HEAD:refs/heads/main"],
        );

        git(&source, &["checkout", "--quiet", "feature"]);
        git(
            &source,
            &["mv", "apps/desktop/src/Old.tsx", "apps/desktop/src/New.tsx"],
        );
        fs::write(
            source.join("apps/desktop/src/App.tsx"),
            "export const App = 1;\n",
        )
        .unwrap();
        git(&source, &["add", "--all"]);
        git(&source, &["commit", "--quiet", "-m", "change feature UI"]);
        let head = git_sha(&source);
        git(
            &source,
            &["push", "--quiet", "origin", "HEAD:refs/heads/feature"],
        );
        git(
            &source,
            &["push", "--quiet", "origin", "HEAD:refs/pull/17/head"],
        );

        let clone = Command::new("git")
            .args([
                "clone",
                "--quiet",
                "--depth=1",
                "--branch",
                "main",
                file_url(&remote).as_str(),
                shallow.to_str().unwrap(),
            ])
            .output()
            .unwrap();
        assert!(
            clone.status.success(),
            "shallow clone failed: {}",
            String::from_utf8_lossy(&clone.stderr)
        );
        git(
            &shallow,
            &["fetch", "--quiet", "--depth=1", "origin", "feature"],
        );
        git(
            &shallow,
            &["checkout", "--quiet", "--detach", head.as_str()],
        );
        let base_expression = format!("{merge_base}^{{commit}}");
        assert!(
            !git_output(&shallow, &["cat-file", "-e", base_expression.as_str()])
                .unwrap()
                .status
                .success()
        );

        let pr_refs = PullRequestRefs {
            number: 17,
            base_ref: "main".to_owned(),
        };
        let paths = changed_paths_from_git(
            &shallow,
            DiffMode::PullRequest,
            &base,
            &head,
            Some(&pr_refs),
        )
        .unwrap();
        assert_eq!(
            paths,
            [
                "apps/desktop/src/App.tsx",
                "apps/desktop/src/New.tsx",
                "apps/desktop/src/Old.tsx",
            ]
        );
        assert!(!paths.iter().any(|path| path == "apps/website/package.json"));
        assert_lanes(
            &classify_git_range_with_refs(
                &shallow,
                DiffMode::PullRequest,
                &base,
                &head,
                Some(&pr_refs),
            ),
            &["desktop_required", "desktop_e2e_required"],
        );
        assert_lanes(
            &classify_git_range(&source, DiffMode::Push, &merge_base, &head),
            &[
                "desktop_required",
                "desktop_e2e_required",
                "package_required",
            ],
        );
        assert!(
            git(&shallow, &["rev-parse", "--abbrev-ref", "HEAD"])
                .stdout
                .starts_with(b"HEAD")
        );

        let mismatch = classify_git_range(&source, DiffMode::PullRequest, &base, &merge_base);
        assert_lanes(&mismatch, &LANES);
        assert!(
            mismatch
                .classification_reason
                .contains("does not match checked-out source HEAD")
        );
        let empty = classify_git_range(&source, DiffMode::Push, &head, &head);
        assert_lanes(&empty, &LANES);
        assert!(
            empty
                .classification_reason
                .contains("empty changed-path diff")
        );
        let unavailable = classify_git_range(
            &source,
            DiffMode::Push,
            "1234567890abcdef1234567890abcdef12345678",
            &head,
        );
        assert_lanes(&unavailable, &LANES);
        assert!(
            unavailable
                .classification_reason
                .contains("could not be fetched")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn fork_head_uses_bounded_pull_request_ref_and_keeps_frontend_pr_narrow() {
        let root = temp_dir("sky-xtask-classifier-fork-pr-ref");
        let source = root.join("source");
        let origin = root.join("origin.git");
        let fork = root.join("fork.git");
        let checkout = root.join("checkout");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&origin).unwrap();
        fs::create_dir(&fork).unwrap();
        git(&origin, &["init", "--bare", "--quiet"]);
        git(&fork, &["init", "--bare", "--quiet"]);
        git(&source, &["init", "--quiet", "--initial-branch=main"]);
        git(&source, &["config", "user.name", "Classifier Test"]);
        git(
            &source,
            &["config", "user.email", "classifier@example.invalid"],
        );
        git(
            &source,
            &["remote", "add", "origin", origin.to_str().unwrap()],
        );
        git(&source, &["remote", "add", "fork", fork.to_str().unwrap()]);
        fs::write(source.join("README.md"), "fork point\n").unwrap();
        git(&source, &["add", "--all"]);
        git(&source, &["commit", "--quiet", "-m", "fork point"]);
        let fork_point = git_sha(&source);
        git(
            &source,
            &["push", "--quiet", "origin", "HEAD:refs/heads/main"],
        );
        git(
            &source,
            &["push", "--quiet", "fork", "HEAD:refs/heads/main"],
        );

        git(&source, &["checkout", "--quiet", "-b", "feature"]);
        fs::create_dir_all(source.join("apps/desktop/src")).unwrap();
        fs::write(
            source.join("apps/desktop/src/App.tsx"),
            "export const App = 2;\n",
        )
        .unwrap();
        git(&source, &["add", "--all"]);
        git(&source, &["commit", "--quiet", "-m", "fork UI change"]);
        let head = git_sha(&source);
        git(
            &source,
            &["push", "--quiet", "fork", "HEAD:refs/heads/feature"],
        );
        git(
            &source,
            &["push", "--quiet", "origin", "HEAD:refs/pull/29/head"],
        );

        git(&source, &["checkout", "--quiet", "main"]);
        fs::write(source.join("README.md"), "main advanced\n").unwrap();
        fs::create_dir_all(source.join("apps/website")).unwrap();
        fs::write(source.join("apps/website/package.json"), "{}\n").unwrap();
        git(&source, &["add", "--all"]);
        git(
            &source,
            &["commit", "--quiet", "-m", "advance upstream after fork"],
        );
        let base = git_sha(&source);
        git(
            &source,
            &["push", "--quiet", "origin", "HEAD:refs/heads/main"],
        );

        let clone = Command::new("git")
            .args([
                "clone",
                "--quiet",
                "--depth=1",
                "--branch",
                "main",
                file_url(&origin).as_str(),
                checkout.to_str().unwrap(),
            ])
            .output()
            .unwrap();
        assert!(
            clone.status.success(),
            "shallow clone failed: {}",
            String::from_utf8_lossy(&clone.stderr)
        );
        git(
            &checkout,
            &["remote", "add", "fork", fork.to_str().unwrap()],
        );
        git(
            &checkout,
            &[
                "fetch",
                "--quiet",
                "--depth=1",
                "fork",
                "refs/heads/feature:refs/remotes/fork/feature",
            ],
        );
        git(
            &checkout,
            &["checkout", "--quiet", "--detach", head.as_str()],
        );

        let advertised = git(&checkout, &["ls-remote", "--heads", "origin"]);
        assert!(!String::from_utf8_lossy(&advertised.stdout).contains(&head));
        assert!(!String::from_utf8_lossy(&advertised.stdout).contains("feature"));
        let pr_refs = PullRequestRefs {
            number: 29,
            base_ref: "main".to_owned(),
        };
        let (base_refspec, pr_head_refspec) = pr_refs.fetch_refspecs(&checkout).unwrap();
        assert_eq!(base_refspec, "+refs/heads/main:refs/sky-ci/base");
        assert_eq!(pr_head_refspec, "+refs/pull/29/head:refs/sky-ci/pr-head");
        assert!(!base_refspec.contains(&head));
        assert!(!pr_head_refspec.contains(&head));

        let paths = changed_paths_from_git(
            &checkout,
            DiffMode::PullRequest,
            &base,
            &head,
            Some(&pr_refs),
        )
        .unwrap();
        assert_eq!(paths, ["apps/desktop/src/App.tsx"]);
        assert_ne!(fork_point, base);
        assert_lanes(
            &classify_git_range_with_refs(
                &checkout,
                DiffMode::PullRequest,
                &base,
                &head,
                Some(&pr_refs),
            ),
            &["desktop_required", "desktop_e2e_required"],
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn shallow_history_without_a_merge_base_fails_closed_after_bounded_fetches() {
        let root = temp_dir("sky-xtask-classifier-no-merge-base");
        let main_repo = root.join("main-repo");
        let other_repo = root.join("other-repo");
        let remote = root.join("origin.git");
        let shallow = root.join("shallow");
        fs::create_dir(&main_repo).unwrap();
        fs::create_dir(&other_repo).unwrap();
        fs::create_dir(&remote).unwrap();
        git(&remote, &["init", "--bare", "--quiet"]);

        git(&main_repo, &["init", "--quiet"]);
        git(&main_repo, &["config", "user.name", "Classifier Test"]);
        git(
            &main_repo,
            &["config", "user.email", "classifier@example.invalid"],
        );
        fs::write(main_repo.join("main.txt"), "main\n").unwrap();
        git(&main_repo, &["add", "--all"]);
        git(&main_repo, &["commit", "--quiet", "-m", "main root"]);
        fs::write(main_repo.join("main.txt"), "main advance\n").unwrap();
        git(&main_repo, &["commit", "--quiet", "-am", "advance main"]);
        let base = git_sha(&main_repo);
        git(
            &main_repo,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
        git(
            &main_repo,
            &["push", "--quiet", "origin", "HEAD:refs/heads/main"],
        );

        git(&other_repo, &["init", "--quiet"]);
        git(&other_repo, &["config", "user.name", "Classifier Test"]);
        git(
            &other_repo,
            &["config", "user.email", "classifier@example.invalid"],
        );
        fs::write(other_repo.join("other.txt"), "unrelated history\n").unwrap();
        git(&other_repo, &["add", "--all"]);
        git(&other_repo, &["commit", "--quiet", "-m", "unrelated root"]);
        let head = git_sha(&other_repo);
        git(
            &other_repo,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
        git(
            &other_repo,
            &["push", "--quiet", "origin", "HEAD:refs/heads/independent"],
        );
        git(
            &other_repo,
            &["push", "--quiet", "origin", "HEAD:refs/pull/17/head"],
        );

        let clone = Command::new("git")
            .args([
                "clone",
                "--quiet",
                "--depth=1",
                "--branch",
                "main",
                file_url(&remote).as_str(),
                shallow.to_str().unwrap(),
            ])
            .output()
            .unwrap();
        assert!(
            clone.status.success(),
            "shallow clone failed: {}",
            String::from_utf8_lossy(&clone.stderr)
        );
        git(
            &shallow,
            &["fetch", "--quiet", "--depth=1", "origin", "independent"],
        );
        git(
            &shallow,
            &["checkout", "--quiet", "--detach", head.as_str()],
        );
        let pr_refs = PullRequestRefs {
            number: 17,
            base_ref: "main".to_owned(),
        };
        let classification = classify_git_range_with_refs(
            &shallow,
            DiffMode::PullRequest,
            &base,
            &head,
            Some(&pr_refs),
        );
        assert_lanes(&classification, &LANES);
        assert!(
            classification
                .classification_reason
                .contains("bounded shallow-history fetch attempts")
        );
        assert_eq!(SHALLOW_DEEPEN_STEPS.len(), 4);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn empty_paths_are_static_and_path_normalization_matches_powershell_inputs() {
        assert_eq!(
            classify_paths(Vec::<String>::new()).classification_reason,
            "no changed paths"
        );
        let normalized = classify_paths([" ./apps\\desktop\\src\\App.tsx ".to_owned()]);
        assert_lanes(&normalized, &["desktop_required", "desktop_e2e_required"]);
        assert!(
            normalized
                .classification_reason
                .ends_with("apps/desktop/src/App.tsx")
        );
    }
}
