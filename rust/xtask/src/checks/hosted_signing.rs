use super::workflow::workflow_job_blocks;
use crate::Result;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

const WORKFLOW_PATH: &str = ".github/workflows/ci.yml";
const JOB_ID: &str = "signing_proof";
const DRAFT_READBACK_STEP: &str = "Run ignored draft readback publication proof";
const DRAFT_READBACK_TEST: &str = "cargo test --locked --manifest-path rust/xtask/Cargo.toml release_v4::tests::test_key_signature_is_verified_after_draft_readback_before_patch -- --ignored --exact --test-threads=1";
const BUILD_STEP: &str = "Build one unsigned NSIS installer without signing credentials";
const SIGN_STEP: &str = "Sign the unchanged installer in an isolated test-key step";
const VERIFY_STEP: &str = "Verify exact test-key signature, version, digest, and source binding";
const CLEANUP_STEP: &str = "Remove synthetic key and hosted proof workspace";

fn step<'a>(source: &'a str, name: &str) -> Result<&'a str> {
    let marker = format!("      - name: {name}\n");
    let start = source
        .find(&marker)
        .ok_or_else(|| format!("signing proof is missing step: {name}"))?;
    let step_source = &source[start + marker.len()..];
    let end = step_source
        .find("\n      - name:")
        .unwrap_or(step_source.len());
    Ok(&step_source[..end])
}

pub(super) fn ci_inherited_read_permissions(source: &str) -> Result<()> {
    let mut in_permissions = false;
    let mut lines = Vec::new();
    for line in source.lines() {
        let indent = line.len() - line.trim_start().len();
        let trimmed = line.trim();
        if !in_permissions {
            if indent == 0 && trimmed == "permissions:" {
                in_permissions = true;
            }
            continue;
        }
        if indent == 0 {
            break;
        }
        if !trimmed.is_empty() {
            lines.push(line.to_owned());
        }
    }
    if !in_permissions || lines.len() != 1 || lines[0] != "  contents: read" {
        return Err("CI inherited permissions must remain contents: read only".into());
    }
    Ok(())
}

fn job(source: &str) -> Result<String> {
    workflow_job_blocks(source)
        .into_iter()
        .find(|(id, _)| id == JOB_ID)
        .map(|(_, block)| block.replace("\r\n", "\n").replace('\r', "\n"))
        .ok_or_else(|| "CI is missing its signing_proof job".into())
}

fn validate_workflow_source(source: &str) -> Result<()> {
    let source = source.replace("\r\n", "\n").replace('\r', "\n");
    ci_inherited_read_permissions(&source)?;
    let job = job(&source)?;
    for marker in [
        "needs: changes",
        "if: needs.changes.outputs.signing_proof_required == 'true'",
        "runs-on: windows-latest",
        "permissions:\n      contents: read",
        "SKY_AUTHENTICODE_MODE: unsigned-zero-budget",
        "SKY_PROOF_SOURCE_SHA: ${{ github.event.pull_request.head.sha || github.sha }}",
        "CARGO_TARGET_DIR=$targetRoot",
        "fetch-depth: 1",
        "persist-credentials: false",
        "uses: ./.github/actions/setup-rust-pinned",
        "bun-version-file: .bun-version",
        "run: bun scripts/verify-bun-pin.mjs",
        "Expected pinned Tauri CLI 2.12.3",
    ] {
        if !job.contains(marker) {
            return Err(format!("signing_proof job is missing required marker: {marker}").into());
        }
    }
    for forbidden in [
        "environment:",
        "secrets:",
        "secrets.",
        "github.token",
        "GH_TOKEN",
        "V4_TAURI_UPDATER_PRIVATE_KEY",
        "V4_RELEASE_METADATA_",
        "v4-production-release",
        "cargo xtask release-v4",
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
        "Swatinem/rust-cache@",
    ] {
        if job.contains(forbidden) {
            return Err(format!(
                "signing_proof job crosses its read-only trust boundary: {forbidden}"
            )
            .into());
        }
    }

    let uses = job
        .lines()
        .filter_map(|line| line.trim().strip_prefix("uses: "))
        .map(|value| value.split('#').next().unwrap_or(value).trim().to_owned())
        .collect::<BTreeSet<_>>();
    let expected_uses = BTreeSet::from([
        "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1".to_owned(),
        "./.github/actions/setup-rust-pinned".to_owned(),
        "oven-sh/setup-bun@0c5077e51419868618aeaa5fe8019c62421857d6".to_owned(),
    ]);
    if uses != expected_uses {
        return Err(format!(
            "signing_proof must use only its pinned read-only setup actions: {uses:?}"
        )
        .into());
    }

    let build = step(&job, BUILD_STEP)?;
    let sign = step(&job, SIGN_STEP)?;
    let verify = step(&job, VERIFY_STEP)?;
    let cleanup = step(&job, CLEANUP_STEP)?;
    let draft_readback = step(&job, DRAFT_READBACK_STEP)?;
    if !draft_readback.contains(DRAFT_READBACK_TEST) {
        return Err("signing_proof must run the ignored draft readback publication proof".into());
    }
    if build.contains("tauri signer")
        || build.contains("--private-key-path")
        || build.contains("TAURI_SIGNING_PRIVATE_KEY=")
        || build.contains("TAURI_SIGNING_PRIVATE_KEY_PATH=")
    {
        return Err("unsigned NSIS build step must not use an updater signing key".into());
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
            return Err(format!("unsigned signing proof build is missing {marker}").into());
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
            return Err(format!("test-key signing step is missing {marker}").into());
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
            return Err(format!("test-key verifier is missing {marker}").into());
        }
    }
    let order = [
        job.find("Install locked desktop dependencies"),
        job.find(DRAFT_READBACK_STEP),
        job.find("Verify source identity and pinned Tauri CLI"),
        job.find(BUILD_STEP),
        job.find(SIGN_STEP),
        job.find(VERIFY_STEP),
        job.find(CLEANUP_STEP),
    ];
    if order.iter().any(Option::is_none)
        || !order.windows(2).all(|pair| pair[0] < pair[1])
        || !job.contains(&format!(
            "      - name: {CLEANUP_STEP}\n        if: always()\n"
        ))
        || !cleanup.contains("RUNNER_TEMP")
        || !cleanup.contains("ownerPath")
        || !cleanup.contains("Remove-Item -LiteralPath $proofRoot -Recurse -Force")
    {
        return Err("signing_proof must build unsigned, sign, verify, then clean its owned RUNNER_TEMP workspace".into());
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
    fn signing_proof_job_is_conditional_isolated_and_unsigned_before_test_key_signing() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let source = fs::read_to_string(root.join(WORKFLOW_PATH))
            .unwrap()
            .replace("\r\n", "\n")
            .replace('\r', "\n");
        validate_workflow_source(&source).unwrap();

        let unrelated_permissions = source.replace(
            "  malware_proof:\n",
            "  unrelated_fixture:\n    permissions:\n      contents: write\n    run: echo secrets.in.other.job\n\n  malware_proof:\n",
        );
        assert!(validate_workflow_source(&unrelated_permissions).is_ok());

        for invalid in [
            source.replace(
                "  contents: read\n",
                "  contents: write\n",
            ),
            source.replace(
                "    if: needs.changes.outputs.signing_proof_required == 'true'\n",
                "    if: always()\n",
            ),
            source.replace("false}}", "true}}"),
            source.replace(
                "          $configPath = Join-Path $env:SKY_HOSTED_PROOF_ROOT",
                "          bun run tauri signer generate --ci --password \"\" --force -w forbidden.key\n          $configPath = Join-Path $env:SKY_HOSTED_PROOF_ROOT",
            ),
            source.replace(
                "      - name: Remove synthetic key and hosted proof workspace\n        if: always()\n",
                "      - name: Remove synthetic key and hosted proof workspace\n",
            ),
            source.replace(
                "      contents: read\n    env:\n      SKY_AUTHENTICODE_MODE",
                "      contents: write\n    env:\n      SKY_AUTHENTICODE_MODE",
            ),
        ] {
            assert_ne!(invalid, source);
            assert!(validate_workflow_source(&invalid).is_err());
        }
    }
}
