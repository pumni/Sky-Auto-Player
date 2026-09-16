#[path = "../../rust/build_support/git_metadata.rs"]
mod git_metadata;

#[allow(dead_code)]
#[path = "src/ipc_contract.rs"]
mod ipc_contract;

fn dirty_worktree() -> bool {
    match std::env::var("SKY_NATIVE_DIRTY_WORKTREE").as_deref() {
        Ok("false") => false,
        Ok("true") => true,
        Ok(_) => true,
        Err(_) => git_metadata::command_output_allow_empty("git", &["status", "--porcelain"])
            .map(|status| !status.is_empty())
            .unwrap_or(true),
    }
}

fn main() {
    println!("cargo:rerun-if-changed=src/ipc_contract.rs");
    git_metadata::emit_git_metadata_rerun_if_changed("../../rust/build_support/git_metadata.rs");
    println!("cargo:rerun-if-env-changed=SKY_CI_SOURCE_SHA");
    println!("cargo:rerun-if-env-changed=GITHUB_SHA");
    println!("cargo:rerun-if-env-changed=SKY_NATIVE_BUILD_COMMIT");
    println!("cargo:rerun-if-env-changed=SKY_NATIVE_DIRTY_WORKTREE");
    println!("cargo:rerun-if-env-changed=SKY_NATIVE_SOURCE_FINGERPRINT");
    let head = std::env::var("SKY_CI_SOURCE_SHA")
        .ok()
        .or_else(|| std::env::var("GITHUB_SHA").ok())
        .or_else(|| std::env::var("SKY_NATIVE_BUILD_COMMIT").ok())
        .or_else(|| git_metadata::command_output("git", &["rev-parse", "--verify", "HEAD"]))
        .unwrap_or_else(|| "unknown".to_string());
    let dirty = dirty_worktree();
    let build_commit = if dirty && !head.ends_with("-dirty") {
        format!("{head}-dirty")
    } else {
        head
    };
    let rustc_version = git_metadata::command_output("rustc", &["--version"])
        .unwrap_or_else(|| "unknown".to_string());
    let source_fingerprint =
        std::env::var("SKY_NATIVE_SOURCE_FINGERPRINT").unwrap_or_else(|_| "unknown".to_string());
    println!("cargo:rustc-env=SKY_NATIVE_BUILD_COMMIT={build_commit}");
    println!("cargo:rustc-env=SKY_NATIVE_DIRTY_WORKTREE={dirty}");
    println!("cargo:rustc-env=SKY_NATIVE_SOURCE_FINGERPRINT={source_fingerprint}");
    println!("cargo:rustc-env=SKY_RUSTC_VERSION={rustc_version}");
    if std::env::var_os("CARGO_FEATURE_TAURI_TEST").is_some() {
        // The MockRuntime-only binding/test validation does not run the
        // frontend. Override only that rustc invocation's generated context
        // so it can use noop assets. Production/default builds keep the real
        // Wry runtime and obtain packaged frontend assets through the
        // packaged-assets feature, which owns tauri/custom-protocol.
        println!("cargo:rustc-env=TAURI_CONFIG={{\"build\":{{\"frontendDist\":null}}}}");
    }
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(ipc_contract::TAURI_COMMANDS)),
    )
    .expect("failed to configure Tauri application command permissions");
}
