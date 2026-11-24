mod bindings;
mod bun_pin;
mod ci_contracts;
mod hosted_malware;
mod hosted_signing;
mod recovery_auth;
mod release_pipeline;
mod release_runner;
mod rust_toolchain;
mod tauri_metadata;
mod trust;
mod workflow;

use crate::{Result, audits, branding, builtin_catalog, process, repo, supply_chain, tauri_bundle};
use serde::Serialize;
use std::path::Path;

pub(crate) use bindings::{bindings, bindings_generate};

pub(super) fn normalize_source_line_endings(source: &str) -> String {
    source.replace("\r\n", "\n").replace('\r', "\n")
}

pub(crate) fn should_skip_supply_chain(flag: bool, env_val: Option<&str>) -> bool {
    flag || env_val.map(|v| v.trim()) == Some("1")
}

pub(crate) const STATIC_LEAF_CHECKS: &[&str] = &[
    "app-boundaries",
    "docs-integrity",
    "architecture",
    "security",
    "tauri-feature-contract",
    "supply-chain",
    "branding",
    "tauri-config",
    "tauri-build-freshness",
    "builtin-catalog",
    "trust-material",
    "bun-pin",
    "rust-toolchain",
    "ci-control-plane",
    "release-metadata",
    "release-runner",
    "hosted-signing",
    "hosted-malware",
    "release-pipeline",
    "packaged-ci",
    "legacy-updater-retirement",
];

pub(crate) const RUST_LEAF_CHECKS: &[&str] =
    &["builtin-catalog", "rust-fmt", "rust-clippy", "rust-tests"];

pub(crate) const DESKTOP_LEAF_CHECKS: &[&str] = &[
    "desktop-install",
    "desktop-check",
    "desktop-e2e",
    "desktop-native",
];

pub(crate) const DESKTOP_NATIVE_LEAF_CHECKS: &[&str] = &["desktop-native"];

fn contract_pointer(check_name: &str) -> Option<&'static str> {
    match check_name {
        "app-boundaries" => Some("docs/architecture.md"),
        "docs-integrity" => Some("docs/INDEX.md"),
        "architecture" => Some("docs/architecture.md"),
        "security" => Some("SECURITY.md"),
        "tauri-feature-contract" => Some("docs/architecture.md"),
        "supply-chain" => Some("rust/supply-chain/config.toml"),
        "branding" => Some("branding/README.md"),
        "tauri-config" => Some("apps/desktop/src-tauri/tauri.conf.json"),
        "tauri-build-freshness" => Some("docs/v4-tauri-packaging.md"),
        "builtin-catalog" => Some("docs/architecture.md"),
        "trust-material" => Some("docs/v4-release-authority.md"),
        "bun-pin" => Some(".bun-version"),
        "rust-toolchain" => Some("rust-toolchain.toml"),
        "ci-control-plane" => Some("docs/ci-validation-topology.md"),
        "release-metadata" => Some("docs/distribution-and-update.md"),
        "release-runner" => Some("docs/v4-release-authority.md"),
        "hosted-signing" => Some("docs/v4-release-authority.md"),
        "hosted-malware" => Some("docs/distribution-and-update.md"),
        "release-pipeline" => Some("docs/v4-release-authority.md"),
        "packaged-ci" => Some("docs/ci-validation-topology.md"),
        "legacy-updater-retirement" => Some("docs/distribution-and-update.md"),
        "rust-fmt" => Some("rust-toolchain.toml"),
        "rust-clippy" => Some("rust-toolchain.toml"),
        "rust-tests" => Some("docs/playback-verification.md"),
        "desktop-install" => Some(".bun-version"),
        "desktop-check" => Some("docs/desktop-tooling.md"),
        "desktop-e2e" => Some("docs/desktop-tooling.md"),
        "desktop-native" => Some("docs/architecture.md"),
        _ => None,
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CheckEntry {
    pub name: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct GroupEntry {
    pub name: String,
    pub status: String,
    pub checks: Vec<CheckEntry>,
}

#[derive(Default)]
pub struct CheckSession {
    pub builtin_catalog_verified: bool,
    pub groups: Vec<GroupEntry>,
    current_group: Option<GroupEntry>,
}

impl CheckSession {
    pub fn start_group(&mut self, name: &'static str, planned_leaves: &[&str]) {
        self.current_group = Some(GroupEntry {
            name: name.to_owned(),
            status: "not-run".to_owned(),
            checks: planned_leaves
                .iter()
                .map(|&leaf| CheckEntry {
                    name: leaf.to_owned(),
                    status: "not-run".to_owned(),
                    reason: None,
                })
                .collect(),
        });
    }

    pub fn record_leaf(&mut self, name: &str, status: &str, reason: Option<String>) {
        if let Some(group) = &mut self.current_group {
            if let Some(entry) = group.checks.iter_mut().find(|c| c.name == name) {
                entry.status = status.to_owned();
                entry.reason = reason;
            } else {
                group.checks.push(CheckEntry {
                    name: name.to_owned(),
                    status: status.to_owned(),
                    reason,
                });
            }
        }
    }

    pub fn finish_group(&mut self, is_ok: bool) {
        if let Some(mut group) = self.current_group.take() {
            group.status = if is_ok {
                "passed".to_owned()
            } else {
                "failed".to_owned()
            };
            self.groups.push(group);
        }
    }

    pub fn add_not_run_group(&mut self, name: &'static str, planned_leaves: &[&str]) {
        self.groups.push(GroupEntry {
            name: name.to_owned(),
            status: "not-run".to_owned(),
            checks: planned_leaves
                .iter()
                .map(|&leaf| CheckEntry {
                    name: leaf.to_owned(),
                    status: "not-run".to_owned(),
                    reason: None,
                })
                .collect(),
        });
    }

    pub fn all_leaf_checks(&self) -> Vec<CheckEntry> {
        let mut checks = Vec::new();
        for g in &self.groups {
            for c in &g.checks {
                checks.push(c.clone());
            }
        }
        checks
    }
}

#[cfg(test)]
fn run_once_per_check_session(
    completed: &mut bool,
    check: impl FnOnce() -> Result<()>,
) -> Result<bool> {
    if *completed {
        return Ok(false);
    }
    check()?;
    *completed = true;
    Ok(true)
}

fn verify_builtin_catalog(root: &Path, session: &mut CheckSession) -> Result<()> {
    if session.builtin_catalog_verified {
        session.record_leaf(
            "builtin-catalog",
            "reused",
            Some("reused-prior-step".to_owned()),
        );
        println!("Built-in catalog verification: PASS (reused in this check session)");
        return Ok(());
    }
    match record_step(session, "builtin-catalog", || {
        builtin_catalog::run(root, "verify", &[])
    }) {
        Ok(()) => {
            session.builtin_catalog_verified = true;
            Ok(())
        }
        Err(e) => Err(e),
    }
}

fn check_desktop_native(root: &Path) -> Result<()> {
    const NATIVE_TAURI_CONFIG: &str = r#"{"build":{"frontendDist":null}}"#;
    let native_env = [("TAURI_CONFIG", NATIVE_TAURI_CONFIG)];
    process::run(
        "cargo",
        &[
            "check",
            "--manifest-path",
            "rust/Cargo.toml",
            "-p",
            "sky_desktop_shell",
            "--bin",
            "sky_desktop_shell",
            "--no-default-features",
            "--features",
            "desktop-runtime",
            "--locked",
        ],
        root,
        &native_env,
    )?;
    process::run(
        "cargo",
        &[
            "check",
            "--manifest-path",
            "rust/Cargo.toml",
            "-p",
            "sky_desktop_shell",
            "--locked",
        ],
        root,
        &native_env,
    )?;
    process::run(
        "cargo",
        &[
            "check",
            "--manifest-path",
            "rust/Cargo.toml",
            "-p",
            "sky_desktop_shell",
            "--all-features",
            "--locked",
        ],
        root,
        &native_env,
    )?;
    bindings::bindings_with_env(&native_env)
}

#[derive(Debug, Clone)]
struct SourceIdentityInfo {
    head: String,
    identity: String,
    dirty: bool,
}

fn inspect_source_identity(root: &Path) -> SourceIdentityInfo {
    let head = repo::git_head(root, false).unwrap_or_else(|_| "unavailable".to_owned());
    let status_output = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(root)
        .output();
    let is_dirty = match status_output {
        Ok(out) => !out.stdout.is_empty(),
        Err(_) => true,
    };
    if is_dirty {
        SourceIdentityInfo {
            head,
            identity: "unqualified-dirty-working-tree".to_owned(),
            dirty: true,
        }
    } else {
        SourceIdentityInfo {
            head: head.clone(),
            identity: head,
            dirty: false,
        }
    }
}

pub fn run(group: &str, skip_supply_chain: bool, json_summary: bool) -> Result<()> {
    let root = repo::root();
    let start_source = inspect_source_identity(&root);
    let start_instant = std::time::Instant::now();

    let mut session = CheckSession::default();
    let result = run_with_session(group, skip_supply_chain, &mut session);

    let elapsed_ms = start_instant.elapsed().as_millis() as u64;
    let end_source = inspect_source_identity(&root);

    let (source_status, source_identity, is_dirty) =
        if start_source.dirty || end_source.dirty || start_source.head != end_source.head {
            let status = if !start_source.dirty && end_source.dirty {
                "modified-during-run"
            } else {
                "dirty"
            };
            (
                status.to_owned(),
                "unqualified-dirty-working-tree".to_owned(),
                true,
            )
        } else {
            ("clean".to_owned(), start_source.identity, false)
        };

    if json_summary {
        let all_checks = session.all_leaf_checks();
        println!(
            "[xtask-summary] {}",
            serde_json::json!({
                "schema_version": 1,
                "group": group,
                "status": if result.is_ok() { "passed" } else { "failed" },
                "duration_ms": elapsed_ms,
                "source_identity": source_identity,
                "source": {
                    "head": start_source.head,
                    "status": source_status,
                    "dirty": is_dirty
                },
                "groups": session.groups,
                "checks": all_checks,
                "unlisted_checks": "not-run or outside requested group",
                "merge_readiness": "not assessed"
            })
        );
    }
    result
}

fn record_step(
    session: &mut CheckSession,
    name: &'static str,
    check: impl FnOnce() -> Result<()>,
) -> Result<()> {
    match check() {
        Ok(()) => {
            session.record_leaf(name, "passed", None);
            Ok(())
        }
        Err(error) => {
            let error_msg = error.to_string();
            eprintln!("[xtask-failure] check: {name}");
            eprintln!("  cause: {error_msg}");
            if let Some(ptr) = contract_pointer(name) {
                eprintln!("  contract pointer: {ptr}");
            }
            let reason_code = match contract_pointer(name) {
                Some(ptr) => format!("check-failed: {ptr}"),
                None => "check-failed".to_owned(),
            };
            session.record_leaf(name, "failed", Some(reason_code));
            Err(error)
        }
    }
}

fn run_with_session(
    group: &str,
    skip_supply_chain: bool,
    session: &mut CheckSession,
) -> Result<()> {
    match group {
        "static" => {
            session.start_group("group-static", STATIC_LEAF_CHECKS);
            let res = run_static_group(skip_supply_chain, session);
            session.finish_group(res.is_ok());
            res
        }
        "rust" => {
            session.start_group("group-rust", RUST_LEAF_CHECKS);
            let res = run_rust_group(session);
            session.finish_group(res.is_ok());
            res
        }
        "desktop" => {
            session.start_group("group-desktop", DESKTOP_LEAF_CHECKS);
            let res = (|| -> Result<()> {
                let root = repo::root();
                record_step(session, "desktop-install", || {
                    process::run(
                        "bun",
                        &["install", "--frozen-lockfile"],
                        &root.join("apps/desktop"),
                        &[],
                    )
                })?;
                record_step(session, "desktop-check", || {
                    process::run("bun", &["run", "check"], &root.join("apps/desktop"), &[])
                })?;
                if std::env::var_os("SKY_DESKTOP_SKIP_BROWSER").is_none() {
                    record_step(session, "desktop-e2e", || {
                        process::run("bun", &["run", "test:e2e"], &root.join("apps/desktop"), &[])
                    })?;
                } else {
                    session.record_leaf(
                        "desktop-e2e",
                        "skipped",
                        Some("SKY_DESKTOP_SKIP_BROWSER set".to_owned()),
                    );
                    println!(
                        "[xtask] desktop browser E2E: SKIP (not required for this validation)"
                    );
                }
                record_step(session, "desktop-native", || check_desktop_native(&root))?;
                println!("[xtask] check desktop: PASS");
                Ok(())
            })();
            session.finish_group(res.is_ok());
            res
        }
        "desktop-native" => {
            session.start_group("group-desktop-native", DESKTOP_NATIVE_LEAF_CHECKS);
            let root = repo::root();
            let res = record_step(session, "desktop-native", || check_desktop_native(&root));
            session.finish_group(res.is_ok());
            if res.is_ok() {
                println!("[xtask] check desktop-native: PASS");
            }
            res
        }
        "all" => {
            session.start_group("group-static", STATIC_LEAF_CHECKS);
            let static_res = run_static_group(skip_supply_chain, session);
            session.finish_group(static_res.is_ok());
            if let Err(e) = static_res {
                session.add_not_run_group("group-rust", RUST_LEAF_CHECKS);
                session.add_not_run_group("group-desktop", DESKTOP_LEAF_CHECKS);
                return Err(e);
            }

            session.start_group("group-rust", RUST_LEAF_CHECKS);
            let rust_res = run_rust_group(session);
            session.finish_group(rust_res.is_ok());
            if let Err(e) = rust_res {
                session.add_not_run_group("group-desktop", DESKTOP_LEAF_CHECKS);
                return Err(e);
            }

            let desktop_res = run_with_session("desktop", skip_supply_chain, session);
            if desktop_res.is_ok() {
                println!("[xtask] check all: PASS");
            }
            desktop_res
        }
        other => Err(format!("unknown check group: {other}").into()),
    }
}

fn run_static_group(skip_supply_chain: bool, session: &mut CheckSession) -> Result<()> {
    let root = repo::root();
    let env_val = std::env::var("SKY_CHECK_SKIP_SUPPLY_CHAIN").ok();
    let skip_supply_chain = should_skip_supply_chain(skip_supply_chain, env_val.as_deref());

    record_step(session, "app-boundaries", || {
        audits::app_boundaries::run(&root)
    })?;
    record_step(session, "docs-integrity", || {
        audits::docs_integrity::run(&root)
    })?;
    record_step(session, "architecture", || audits::architecture::run(&root))?;
    record_step(session, "security", || audits::security::run(&root))?;
    record_step(session, "tauri-feature-contract", || {
        tauri_metadata::tauri_feature_contract(&root)
    })?;
    if !skip_supply_chain {
        record_step(session, "supply-chain", || supply_chain::run(None))?;
    } else {
        session.record_leaf(
            "supply-chain",
            "skipped",
            Some("verified by dedicated supply-chain gate or explicit skip".to_owned()),
        );
        println!("[xtask] cargo-vet supply-chain: SKIP (verified by dedicated supply-chain gate)");
    }
    record_step(session, "branding", || branding::validate(&root))?;
    record_step(session, "tauri-config", || {
        tauri_bundle::validate_config(&root)
    })?;
    record_step(session, "tauri-build-freshness", || {
        tauri_metadata::tauri_build_freshness_contract(&root)
    })?;
    verify_builtin_catalog(&root, session)?;
    record_step(session, "trust-material", || {
        trust::v4_trust_material_contract(&root)
    })?;
    record_step(session, "bun-pin", || bun_pin::bun_pin_contract(&root))?;
    record_step(session, "rust-toolchain", || {
        rust_toolchain::rust_toolchain_contract(&root)
    })?;
    record_step(session, "ci-control-plane", || {
        ci_contracts::ci_control_plane_contract(&root)
    })?;
    record_step(session, "release-metadata", || {
        tauri_metadata::release_metadata_contract(&root)
    })?;
    record_step(session, "release-runner", || {
        release_runner::release_runner_contract(&root)
    })?;
    record_step(session, "hosted-signing", || {
        hosted_signing::hosted_v4_signing_workflow_contract(&root)
    })?;
    record_step(session, "hosted-malware", || {
        hosted_malware::hosted_malware_proof_contract(&root)
    })?;
    record_step(session, "release-pipeline", || {
        release_pipeline::v4_release_pipeline_contract(&root)
    })?;
    record_step(session, "packaged-ci", || {
        ci_contracts::packaged_ci_contract(&root)
    })?;
    record_step(session, "legacy-updater-retirement", || {
        tauri_metadata::v4_legacy_updater_retirement(&root)
    })?;
    println!("[xtask] check static: PASS");
    Ok(())
}

fn run_rust_group(session: &mut CheckSession) -> Result<()> {
    let root = repo::root();
    verify_builtin_catalog(&root, session)?;

    let export_dir = bindings::prepare_binding_export_dir(&root)?;
    let export_dir = export_dir
        .to_str()
        .ok_or("binding export directory is not valid UTF-8")?
        .to_owned();
    let test_env = [
        ("CARGO_TARGET_DIR", "rust/target/check-all-rust-tests"),
        ("RUST_TEST_THREADS", "1"),
        ("TS_RS_EXPORT_DIR", export_dir.as_str()),
    ];

    record_step(session, "rust-fmt", || {
        process::run(
            "cargo",
            &[
                "fmt",
                "--manifest-path",
                "rust/Cargo.toml",
                "--all",
                "--",
                "--check",
            ],
            &root,
            &[],
        )
    })?;

    record_step(session, "rust-clippy", || {
        process::run(
            "cargo",
            &[
                "clippy",
                "--manifest-path",
                "rust/Cargo.toml",
                "--workspace",
                "--all-targets",
                "--all-features",
                "--locked",
                "--",
                "-D",
                "warnings",
            ],
            &root,
            &[],
        )
    })?;

    record_step(session, "rust-tests", || {
        process::run(
            "cargo",
            &[
                "test",
                "--manifest-path",
                "rust/Cargo.toml",
                "--workspace",
                "--all-features",
                "--locked",
            ],
            &root,
            &test_env,
        )
    })?;

    println!("[xtask] check rust: PASS");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_session_reuses_only_a_successful_verification() {
        let mut completed = false;
        let attempts = std::cell::Cell::new(0);

        assert!(
            run_once_per_check_session(&mut completed, || {
                attempts.set(attempts.get() + 1);
                Ok(())
            })
            .unwrap()
        );
        assert!(
            !run_once_per_check_session(&mut completed, || {
                attempts.set(attempts.get() + 1);
                Ok(())
            })
            .unwrap()
        );
        assert_eq!(attempts.get(), 1);

        let mut independent_session = CheckSession::default();
        assert!(
            run_once_per_check_session(&mut independent_session.builtin_catalog_verified, || {
                attempts.set(attempts.get() + 1);
                Ok(())
            })
            .unwrap()
        );
        assert_eq!(attempts.get(), 2);
    }

    #[test]
    fn failed_check_session_verification_is_retried() {
        let mut completed = false;
        let failed = run_once_per_check_session(&mut completed, || {
            Err(std::io::Error::other("fixture failure").into())
        });
        assert!(failed.is_err());
        assert!(!completed);

        assert!(run_once_per_check_session(&mut completed, || Ok(())).unwrap());
        assert!(completed);
    }

    #[test]
    fn should_skip_supply_chain_fails_closed() {
        assert!(!should_skip_supply_chain(false, None));
        assert!(should_skip_supply_chain(true, None));
        assert!(should_skip_supply_chain(true, Some("0")));
        assert!(should_skip_supply_chain(true, Some("false")));
        assert!(should_skip_supply_chain(false, Some("1")));
        assert!(should_skip_supply_chain(false, Some(" 1 ")));
        assert!(!should_skip_supply_chain(false, Some("0")));
        assert!(!should_skip_supply_chain(false, Some("false")));
        assert!(!should_skip_supply_chain(false, Some("FALSE")));
        assert!(!should_skip_supply_chain(false, Some("true")));
        assert!(!should_skip_supply_chain(false, Some("")));
        assert!(!should_skip_supply_chain(false, Some("yes")));
        assert!(!should_skip_supply_chain(false, Some("2")));
    }

    #[test]
    fn check_session_records_leaves_failures_and_not_run() {
        let mut session = CheckSession::default();
        session.start_group("group-test", &["step-a", "step-b", "step-c"]);

        assert!(record_step(&mut session, "step-a", || Ok(())).is_ok());
        let fail = record_step(&mut session, "step-b", || Err("injected error".into()));
        assert!(fail.is_err());
        session.finish_group(false);

        let group = &session.groups[0];
        assert_eq!(group.name, "group-test");
        assert_eq!(group.status, "failed");
        assert_eq!(group.checks.len(), 3);

        assert_eq!(group.checks[0].name, "step-a");
        assert_eq!(group.checks[0].status, "passed");

        assert_eq!(group.checks[1].name, "step-b");
        assert_eq!(group.checks[1].status, "failed");
        assert_eq!(group.checks[1].reason.as_deref(), Some("check-failed"));

        assert_eq!(group.checks[2].name, "step-c");
        assert_eq!(group.checks[2].status, "not-run");
    }

    #[test]
    fn check_session_summary_schema_free_of_secrets_and_absolute_paths() {
        let mut session = CheckSession::default();
        session.start_group("group-static", STATIC_LEAF_CHECKS);

        // Inject failure containing absolute path and secret credentials
        let sensitive_err = "failed at C:\\Users\\Administrator\\secret_token\\private.key with password=SuperSecret123!";
        let _ = record_step(&mut session, "app-boundaries", || Err(sensitive_err.into()));
        session.record_leaf(
            "supply-chain",
            "skipped",
            Some("supply-chain-skipped".to_owned()),
        );
        session.finish_group(false);

        let json = serde_json::to_string(&session.groups).unwrap();
        assert!(!json.contains("Administrator"));
        assert!(!json.contains("secret_token"));
        assert!(!json.contains("SuperSecret123"));
        assert!(!json.contains("C:\\"));
        assert!(!json.contains("D:\\"));
        assert!(json.contains("group-static"));
        assert!(json.contains("app-boundaries"));
        assert!(json.contains("check-failed: docs/architecture.md"));
        assert!(json.contains("supply-chain"));
        assert!(json.contains("supply-chain-skipped"));
    }

    #[test]
    fn verify_builtin_catalog_failure_is_recorded_as_failed() {
        let mut session = CheckSession::default();
        session.start_group("group-rust", RUST_LEAF_CHECKS);

        // Simulate catalog verify failure
        let res = record_step(&mut session, "builtin-catalog", || {
            Err("corrupted catalog manifest".into())
        });
        assert!(res.is_err());
        session.finish_group(false);

        let group = &session.groups[0];
        let catalog_check = group
            .checks
            .iter()
            .find(|c| c.name == "builtin-catalog")
            .unwrap();
        assert_eq!(catalog_check.status, "failed");
        assert_eq!(
            catalog_check.reason.as_deref(),
            Some("check-failed: docs/architecture.md")
        );
    }

    #[test]
    fn contract_pointers_defined_for_critical_checks() {
        assert_eq!(
            contract_pointer("app-boundaries"),
            Some("docs/architecture.md")
        );
        assert_eq!(contract_pointer("security"), Some("SECURITY.md"));
        assert_eq!(
            contract_pointer("desktop-native"),
            Some("docs/architecture.md")
        );
        assert_eq!(
            contract_pointer("desktop-e2e"),
            Some("docs/desktop-tooling.md")
        );
    }
}
