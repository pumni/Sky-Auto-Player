mod bindings;
mod ci_contracts;
mod hosted_signing;
mod recovery_auth;
mod release_pipeline;
mod release_runner;
mod tauri_metadata;
mod trust;
mod workflow;

use crate::{Result, audits, branding, builtin_catalog, process, repo, supply_chain, tauri_bundle};
use std::path::Path;

pub(crate) use bindings::{bindings, bindings_generate};

pub(super) fn normalize_source_line_endings(source: &str) -> String {
    source.replace("\r\n", "\n").replace('\r', "\n")
}

pub(crate) fn should_skip_supply_chain(flag: bool, env_val: Option<&str>) -> bool {
    flag || env_val.map(|v| v.trim()) == Some("1")
}

#[derive(Default)]
struct CheckSession {
    builtin_catalog_verified: bool,
}

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
    let ran = run_once_per_check_session(&mut session.builtin_catalog_verified, || {
        builtin_catalog::run(root, "verify", &[])
    })?;
    if !ran {
        println!("Built-in catalog verification: PASS (reused in this check session)");
    }
    Ok(())
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

pub fn run(group: &str, skip_supply_chain: bool) -> Result<()> {
    run_with_session(group, skip_supply_chain, &mut CheckSession::default())
}

fn run_with_session(
    group: &str,
    skip_supply_chain: bool,
    session: &mut CheckSession,
) -> Result<()> {
    let root = repo::root();
    let env_val = std::env::var("SKY_CHECK_SKIP_SUPPLY_CHAIN").ok();
    let skip_supply_chain = should_skip_supply_chain(skip_supply_chain, env_val.as_deref());
    match group {
        "static" => {
            audits::docs_integrity::run(&root)?;
            audits::durable_names::run(&root)?;
            audits::legacy_compatibility_names::run(&root)?;
            audits::architecture::run(&root)?;
            audits::security::run(&root)?;
            tauri_metadata::tauri_feature_contract(&root)?;
            if !skip_supply_chain {
                supply_chain::run(None)?;
            } else {
                println!(
                    "[xtask] cargo-vet supply-chain: SKIP (verified by dedicated supply-chain gate)"
                );
            }
            branding::validate(&root)?;
            tauri_bundle::validate_config(&root)?;
            tauri_metadata::tauri_build_freshness_contract(&root)?;
            verify_builtin_catalog(&root, session)?;
            trust::v4_trust_material_contract(&root)?;
            ci_contracts::ci_control_plane_contract(&root)?;
            tauri_metadata::release_metadata_contract(&root)?;
            release_runner::release_runner_contract(&root)?;
            hosted_signing::hosted_v4_signing_workflow_contract(&root)?;
            release_pipeline::v4_release_pipeline_contract(&root)?;
            ci_contracts::packaged_ci_contract(&root)?;
            tauri_metadata::v4_legacy_updater_retirement(&root)?;
        }
        "rust" => {
            verify_builtin_catalog(&root, session)?;
            // The canonical Windows qualification runs workspace tests in a
            // restricted environment.  Keep process-global test fixtures
            // deterministic there; this does not change product concurrency.
            let export_dir = bindings::prepare_binding_export_dir(&root)?;
            let export_dir = export_dir
                .to_str()
                .ok_or("binding export directory is not valid UTF-8")?
                .to_owned();
            let test_env = [
                // Cargo must not replace the running xtask.exe in the shared target directory.
                ("CARGO_TARGET_DIR", "rust/target/check-all-rust-tests"),
                ("RUST_TEST_THREADS", "1"),
                ("TS_RS_EXPORT_DIR", export_dir.as_str()),
            ];
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
            )?;
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
            )?;
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
            )?;
        }
        "desktop" => {
            process::run(
                "bun",
                &["install", "--frozen-lockfile"],
                &root.join("desktop"),
                &[],
            )?;
            process::run("bun", &["run", "check"], &root.join("desktop"), &[])?;
            if std::env::var_os("SKY_DESKTOP_SKIP_BROWSER").is_none() {
                process::run("bun", &["run", "test:e2e"], &root.join("desktop"), &[])?;
            } else {
                println!("[xtask] desktop browser E2E: SKIP (not required for this validation)");
            }
            check_desktop_native(&root)?;
        }
        "desktop-native" => {
            check_desktop_native(&root)?;
        }
        "all" => {
            run_with_session("static", skip_supply_chain, session)?;
            run_with_session("rust", skip_supply_chain, session)?;
            run_with_session("desktop", skip_supply_chain, session)?;
        }
        other => return Err(format!("unknown check group: {other}").into()),
    }
    println!("[xtask] check {group}: PASS");
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
        // Absent env var and flag false -> do not skip
        assert!(!should_skip_supply_chain(false, None));

        // Explicit flag true -> skip
        assert!(should_skip_supply_chain(true, None));
        assert!(should_skip_supply_chain(true, Some("0")));
        assert!(should_skip_supply_chain(true, Some("false")));

        // Env var exactly "1" -> skip
        assert!(should_skip_supply_chain(false, Some("1")));
        assert!(should_skip_supply_chain(false, Some(" 1 ")));

        // Ambiguous / falsey / arbitrary env vars -> fail closed (do NOT skip)
        assert!(!should_skip_supply_chain(false, Some("0")));
        assert!(!should_skip_supply_chain(false, Some("false")));
        assert!(!should_skip_supply_chain(false, Some("FALSE")));
        assert!(!should_skip_supply_chain(false, Some("true")));
        assert!(!should_skip_supply_chain(false, Some("")));
        assert!(!should_skip_supply_chain(false, Some("yes")));
        assert!(!should_skip_supply_chain(false, Some("2")));
    }
}
