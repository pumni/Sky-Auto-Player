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

fn native_abi() -> String {
    if let Ok(value) = std::env::var("SKY_NATIVE_ABI")
        && !value.trim().is_empty()
    {
        return value;
    }
    let python = std::env::var("PYO3_PYTHON").unwrap_or_else(|_| "python".to_string());
    command_output(
        &python,
        &[
            "-c",
            "import sys, sysconfig; abi = (sysconfig.get_config_var('SOABI') or sys.implementation.cache_tag).split('-')[0]; print(f'{abi}-{sysconfig.get_platform().replace('-', '_')}')",
        ],
    )
    .unwrap_or_else(|| "unknown".to_string())
}

fn main() {
    println!("cargo:rerun-if-env-changed=GITHUB_SHA");
    println!("cargo:rerun-if-env-changed=SKY_NATIVE_BUILD_COMMIT");
    println!("cargo:rerun-if-env-changed=SKY_NATIVE_ABI");

    let rustc_version =
        command_output("rustc", &["--version"]).unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=SKY_RUSTC_VERSION={rustc_version}");

    let build_commit = std::env::var("SKY_NATIVE_BUILD_COMMIT")
        .ok()
        .or_else(|| std::env::var("GITHUB_SHA").ok())
        .or_else(|| command_output("git", &["rev-parse", "--verify", "HEAD"]))
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=SKY_NATIVE_BUILD_COMMIT={build_commit}");
    println!("cargo:rustc-env=SKY_NATIVE_ABI={}", native_abi());
}
