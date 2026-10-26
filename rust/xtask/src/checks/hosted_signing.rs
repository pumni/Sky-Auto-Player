use crate::Result;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

const WORKFLOW_PATH: &str = ".github/workflows/hosted-v4-signing-proof.yml";
const BUILD_STEP: &str = "Build one unsigned NSIS installer without signing credentials";
const SIGN_STEP: &str = "Sign the unchanged installer in an isolated test-key step";
const VERIFY_STEP: &str = "Verify exact test-key signature, version, digest, and source binding";
const CLEANUP_STEP: &str = "Remove synthetic key and hosted proof workspace";

fn step<'a>(source: &'a str, name: &str) -> Result<&'a str> {
    let marker = format!("      - name: {name}\n");
    let start = source
        .find(&marker)
        .ok_or_else(|| format!("hosted signing proof workflow is missing step: {name}"))?;
    let step_source = &source[start + marker.len()..];
    let end = step_source
        .find("\n      - name:")
        .unwrap_or(step_source.len());
    Ok(&step_source[..end])
}

fn validate_workflow_source(source: &str) -> Result<()> {
    let normalized_source = source.replace("\r\n", "\n").replace('\r', "\n");
    let source = normalized_source.as_str();
    for marker in [
        "  pull_request:\n",
        "  workflow_dispatch:\n",
        "permissions:\n  contents: read\n",
        "    runs-on: windows-latest\n",
        "persist-credentials: false",
        "SKY_PROOF_SOURCE_SHA: ${{ github.event.pull_request.head.sha || github.sha }}",
        "RUSTUP_TOOLCHAIN: 1.99.1",
        "bun-version: 1.4.3",
        "Expected pinned Tauri CLI 2.12.3",
        "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1",
        "oven-sh/setup-bun@0c5077e51419868618aeaa5fe8019c62421857d6",
    ] {
        if !source.contains(marker) {
            return Err(format!(
                "hosted signing proof workflow is missing required trust-boundary marker: {marker}"
            )
            .into());
        }
    }
    for forbidden in [
        "pull_request_target:",
        "push:",
        "environment:",
        "secrets:",
        "secrets.",
        "github.token",
        "GH_TOKEN",
        "V4_UPDATER_PRIVATE_KEY_PATH",
        "V4_RELEASE_METADATA_",
        "v4-production-release",
        "credential run-action",
        "cargo xtask release-v4",
        "release-metadata",
        "gh api",
        "gh release",
        "Invoke-RestMethod",
        "Invoke-WebRequest",
        "id-token: write",
        "contents: write",
        "actions: write",
        "attestations: write",
        "actions/upload-artifact@",
        "actions/cache@",
    ] {
        if source.contains(forbidden) {
            return Err(format!(
                "hosted signing proof workflow must remain credential-free and externally read only; found {forbidden}"
            )
            .into());
        }
    }

    let uses = source
        .lines()
        .filter_map(|line| line.trim().strip_prefix("uses: "))
        .map(|value| value.split('#').next().unwrap_or(value).trim().to_owned())
        .collect::<BTreeSet<_>>();
    let expected_uses = BTreeSet::from([
        "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1".to_owned(),
        "oven-sh/setup-bun@0c5077e51419868618aeaa5fe8019c62421857d6".to_owned(),
    ]);
    if uses != expected_uses {
        return Err(format!(
            "hosted signing proof workflow must use only its pinned read-only setup actions: {uses:?}"
        )
        .into());
    }

    let build = step(source, BUILD_STEP)?;
    let sign = step(source, SIGN_STEP)?;
    let verify = step(source, VERIFY_STEP)?;
    let cleanup = step(source, CLEANUP_STEP)?;
    if build.contains("tauri signer")
        || build.contains("--private-key-path")
        || build.contains("TAURI_SIGNING_PRIVATE_KEY=")
        || build.contains("TAURI_SIGNING_PRIVATE_KEY_PATH=")
    {
        return Err(
            "hosted proof build step must not receive or use any updater signing key".into(),
        );
    }
    for marker in [
        "createUpdaterArtifacts",
        "false}}",
        "TAURI_SIGNING_PRIVATE_KEY",
        "TAURI_SIGNING_PRIVATE_KEY_PATH",
        "TAURI_SIGNING_PRIVATE_KEY_PASSWORD",
        "files.Count -ne 1",
        "Get-FileHash",
        "SKY_PROOF_INSTALLER_SHA256",
    ] {
        if !build.contains(marker) {
            return Err(format!(
                "hosted proof unsigned build step is missing required marker: {marker}"
            )
            .into());
        }
    }
    for marker in [
        "bun run tauri signer generate",
        "synthetic-updater.key",
        "bun run tauri signer sign",
        "--private-key-path",
        "--app-version",
        "SKY_PROOF_INSTALLER_SHA256",
    ] {
        if !sign.contains(marker) {
            return Err(
                format!("hosted proof signer step is missing required marker: {marker}").into(),
            );
        }
    }
    for marker in [
        "cargo xtask updater-trust verify-test-signature",
        "--expected-version",
        "--expected-sha256",
        "--source-sha",
        "--evidence",
    ] {
        if !verify.contains(marker) {
            return Err(
                format!("hosted proof verifier step is missing required marker: {marker}").into(),
            );
        }
    }
    let build_position = source.find(BUILD_STEP).unwrap();
    let sign_position = source.find(SIGN_STEP).unwrap();
    let verify_position = source.find(VERIFY_STEP).unwrap();
    let cleanup_position = source.find(CLEANUP_STEP).unwrap();
    if !(build_position < sign_position
        && sign_position < verify_position
        && verify_position < cleanup_position)
    {
        return Err("hosted proof must build, sign, verify, then clean up in that order".into());
    }
    if !source.contains(&format!(
        "      - name: {CLEANUP_STEP}\n        if: always()\n"
    )) || !cleanup.contains("RUNNER_TEMP")
        || !cleanup.contains("ownerPath")
        || !cleanup.contains("Remove-Item -LiteralPath $proofRoot -Recurse -Force")
    {
        return Err(
            "hosted proof must always remove only its owned runner-local test workspace".into(),
        );
    }
    Ok(())
}

pub(crate) fn hosted_v4_signing_workflow_contract(root: &Path) -> Result<()> {
    let source = fs::read_to_string(root.join(WORKFLOW_PATH))?;
    validate_workflow_source(&source)?;
    println!("[xtask] hosted v4 test-key signing proof boundary: PASS");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosted_signing_workflow_is_read_only_and_builds_before_test_key_signing() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let source = fs::read_to_string(root.join(WORKFLOW_PATH))
            .unwrap()
            .replace("\r\n", "\n")
            .replace('\r', "\n");
        validate_workflow_source(&source).unwrap();

        let secret_reference = source.replace(
            "  contents: read\n",
            "  contents: read\n  secrets: inherit\n",
        );
        assert!(validate_workflow_source(&secret_reference).is_err());

        let write_permission = source.replace("  contents: read\n", "  contents: write\n");
        assert!(validate_workflow_source(&write_permission).is_err());

        let production_trigger = source.replace("  pull_request:\n", "  pull_request_target:\n");
        assert!(validate_workflow_source(&production_trigger).is_err());

        let signed_build = source.replace("false}}", "true}}");
        assert!(validate_workflow_source(&signed_build).is_err());

        let signing_in_build = source.replace(
            "          $configPath = Join-Path $env:SKY_HOSTED_PROOF_ROOT",
            "          bun run tauri signer generate --ci --password \"\" --force -w forbidden.key\n          $configPath = Join-Path $env:SKY_HOSTED_PROOF_ROOT",
        );
        assert!(validate_workflow_source(&signing_in_build).is_err());

        let no_failure_cleanup = source.replace(
            "      - name: Remove synthetic key and hosted proof workspace\n        if: always()\n",
            "      - name: Remove synthetic key and hosted proof workspace\n",
        );
        assert!(validate_workflow_source(&no_failure_cleanup).is_err());
    }
}
