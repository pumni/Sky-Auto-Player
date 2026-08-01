use std::process::Command;

fn command_output(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?;
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn main() {
    println!("cargo:rerun-if-env-changed=GITHUB_SHA");
    println!("cargo:rerun-if-env-changed=SKY_NATIVE_BUILD_COMMIT");
    println!("cargo:rerun-if-env-changed=SKY_NATIVE_SOURCE_FINGERPRINT");

    let head = std::env::var("GITHUB_SHA")
        .ok()
        .or_else(|| std::env::var("SKY_NATIVE_BUILD_COMMIT").ok())
        .or_else(|| command_output("git", &["rev-parse", "--verify", "HEAD"]))
        .unwrap_or_else(|| "unknown".to_string());
    let dirty = command_output("git", &["status", "--porcelain"])
        .map(|status| !status.is_empty())
        .unwrap_or(false);
    let build_commit = if dirty && !head.ends_with("-dirty") {
        format!("{head}-dirty")
    } else {
        head
    };
    let rustc_version =
        command_output("rustc", &["--version"]).unwrap_or_else(|| "unknown".to_string());
    let source_fingerprint =
        std::env::var("SKY_NATIVE_SOURCE_FINGERPRINT").unwrap_or_else(|_| "unknown".to_string());

    println!("cargo:rustc-env=SKY_NATIVE_BUILD_COMMIT={build_commit}");
    println!("cargo:rustc-env=SKY_NATIVE_SOURCE_FINGERPRINT={source_fingerprint}");
    println!("cargo:rustc-env=SKY_RUSTC_VERSION={rustc_version}");
}
