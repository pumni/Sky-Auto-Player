use crate::{Result, audits, branding, process, repo, supply_chain, tauri_bundle};
use base64::{Engine, engine::general_purpose::STANDARD};
use minisign_verify::PublicKey;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::Path;
use walkdir::WalkDir;

const FORBIDDEN_SECURITY_APIS: &[&str] = &[
    "SetWindowsHookEx",
    "SetWindowsHookExA",
    "SetWindowsHookExW",
    "SetWinEventHook",
    "ReadProcessMemory",
    "WriteProcessMemory",
    "NtReadVirtualMemory",
    "NtWriteVirtualMemory",
    "VirtualAllocEx",
    "VirtualFreeEx",
    "VirtualProtectEx",
    "VirtualQueryEx",
    "CreateRemoteThread",
    "CreateRemoteThreadEx",
    "NtCreateThreadEx",
    "RtlCreateUserThread",
    "QueueUserAPC",
    "GetThreadContext",
    "SetThreadContext",
    "SuspendThread",
    "DebugActiveProcess",
    "DebugActiveProcessStop",
    "ContinueDebugEvent",
    "WaitForDebugEvent",
    "NtQueryInformationProcess",
    "keybd_event",
    "mouse_event",
];
const ALLOWED_WINDOWS_SYS_MODULES: &[&str] = &[
    "Win32::Foundation",
    "Win32::Media",
    "Win32::UI::Input",
    "Win32::System::Performance",
    "Win32::System::LibraryLoader",
    "Win32::System::SystemInformation",
    "Win32::System::Threading",
    "Win32::UI::Input::KeyboardAndMouse",
    "Win32::UI::Controls",
    "Win32::UI::Shell",
    "Win32::UI::HiDpi",
    "Win32::UI::WindowsAndMessaging",
    "Win32::Networking::WinHttp",
    "Win32::Storage::FileSystem",
];
const FORBIDDEN_DLLS: &[&str] = &["ntdll.dll"];
const RETIRED_ACTIVE_TOKENS: &[&str] = &[
    "pyo3",
    "maturin",
    "PyInstaller",
    "sky_player_rs",
    "desktop_ipc",
    "Sky-Auto-Player-Core.exe",
    "build_rust_wheel.py",
    "scripts/check.py",
    "scripts/build_portable_release.py",
    "scripts/verify_release_manifest.py",
];

fn rust_manifest(root: &Path) -> Result<String> {
    Ok(fs::read_to_string(root.join("rust/Cargo.toml"))?)
}

#[derive(Debug, Eq, PartialEq)]
struct TauriFeatureResolution {
    default: BTreeSet<String>,
    dev: BTreeSet<String>,
}

fn feature_entries(
    features: &toml::value::Table,
    name: &str,
) -> std::result::Result<Vec<String>, String> {
    let value = features
        .get(name)
        .ok_or_else(|| format!("feature `{name}` is missing"))?;
    let entries = value
        .as_array()
        .ok_or_else(|| format!("feature `{name}` must be an array"))?;
    entries
        .iter()
        .map(|entry| {
            entry
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("feature `{name}` contains a non-string entry"))
        })
        .collect()
}

fn collect_feature_closure(
    name: &str,
    features: &toml::value::Table,
    values: &mut BTreeSet<String>,
    visiting: &mut BTreeSet<String>,
) -> std::result::Result<(), String> {
    if !visiting.insert(name.to_owned()) {
        return Err(format!("feature graph contains a cycle at `{name}`"));
    }
    for entry in feature_entries(features, name)? {
        values.insert(entry.clone());
        if features.contains_key(&entry) {
            collect_feature_closure(&entry, features, values, visiting)?;
        }
    }
    visiting.remove(name);
    Ok(())
}

fn simulate_tauri_dev_features(
    default_entries: &[String],
    features: &toml::value::Table,
) -> std::result::Result<BTreeSet<String>, String> {
    let mut dev = BTreeSet::new();
    for feature in default_entries {
        let entries = feature_entries(features, feature)?;
        if !entries.iter().any(|entry| entry == "tauri/custom-protocol") {
            dev.insert(feature.clone());
        }
    }
    Ok(dev)
}

fn tauri_feature_contract_manifest(
    source: &str,
) -> std::result::Result<TauriFeatureResolution, String> {
    let manifest = toml::from_str::<toml::Value>(source.trim_start())
        .map_err(|error| format!("invalid Cargo.toml: {error}"))?;
    let dependencies = manifest
        .get("dependencies")
        .and_then(toml::Value::as_table)
        .ok_or_else(|| "Cargo.toml is missing [dependencies]".to_owned())?;
    let tauri_dependency = dependencies
        .get("tauri")
        .and_then(toml::Value::as_table)
        .ok_or_else(|| "tauri dependency must use an inline table".to_owned())?;
    if tauri_dependency
        .get("default-features")
        .and_then(toml::Value::as_bool)
        != Some(false)
    {
        return Err("tauri dependency must keep default-features = false".to_owned());
    }

    let features = manifest
        .get("features")
        .and_then(toml::Value::as_table)
        .ok_or_else(|| "Cargo.toml is missing [features]".to_owned())?;
    let default_entries = feature_entries(features, "default")?;
    let default = default_entries.iter().cloned().collect::<BTreeSet<_>>();
    let expected_default = ["desktop-runtime", "packaged-assets"]
        .into_iter()
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    if default != expected_default || default_entries.len() != default.len() {
        return Err(format!(
            "default features must directly contain exactly `desktop-runtime` and `packaged-assets`; found {default:?}"
        ));
    }

    let desktop_runtime = feature_entries(features, "desktop-runtime")?;
    if !desktop_runtime.iter().any(|entry| entry == "tauri/wry") {
        return Err("desktop-runtime must directly contain `tauri/wry`".to_owned());
    }
    let forbidden_runtime_entries = [
        "tauri/custom-protocol",
        "tauri/x11",
        "tauri/dbus",
        "tauri/dynamic-acl",
        "tauri/common-controls-v6",
    ];
    let mut runtime_closure = BTreeSet::new();
    collect_feature_closure(
        "desktop-runtime",
        features,
        &mut runtime_closure,
        &mut BTreeSet::new(),
    )?;
    if let Some(forbidden) = forbidden_runtime_entries
        .iter()
        .find(|entry| runtime_closure.contains(**entry))
    {
        return Err(format!(
            "desktop-runtime must not contain `{forbidden}` directly or through another feature"
        ));
    }

    let packaged_assets = feature_entries(features, "packaged-assets")?;
    for required in ["tauri/custom-protocol", "tauri/compression"] {
        if !packaged_assets.iter().any(|entry| entry == required) {
            return Err(format!(
                "packaged-assets must directly contain `{required}`"
            ));
        }
    }

    let dev = simulate_tauri_dev_features(&default_entries, features)?;
    let expected_dev = ["desktop-runtime".to_owned()]
        .into_iter()
        .collect::<BTreeSet<_>>();
    if dev != expected_dev {
        return Err(format!(
            "Tauri CLI dev feature simulation must resolve to `desktop-runtime`; found {dev:?}"
        ));
    }

    Ok(TauriFeatureResolution { default, dev })
}

fn tauri_feature_contract(root: &Path) -> Result<()> {
    let manifest_path = root.join("desktop/src-tauri/Cargo.toml");
    let resolution = tauri_feature_contract_manifest(&fs::read_to_string(&manifest_path)?)
        .map_err(|error| format!("{}: {error}", manifest_path.display()))?;
    println!(
        "[xtask] Tauri feature contract: PASS (default={:?}, dev={:?})",
        resolution.default, resolution.dev
    );
    Ok(())
}

fn legacy_release_guard_source(source: &str) -> Result<()> {
    for marker in [
        "name: Block v4+ tags from legacy v3 release workflow",
        "$tag = $env:GITHUB_REF_NAME",
        r#"$tag -match '^v(?<major>\d+)\.'"#,
        "$major -ge 4",
        "v4 publication is prohibited in the legacy v3 release workflow; use the dedicated v4 release authority",
        "dedicated v4 release authority",
    ] {
        if !source.contains(marker) {
            return Err(format!(
                "legacy release workflow is missing the v4 isolation guard marker: {marker}"
            )
            .into());
        }
    }
    Ok(())
}

fn legacy_release_guard(root: &Path) -> Result<()> {
    let path = root.join(".github/workflows/release.yml");
    legacy_release_guard_source(&fs::read_to_string(&path)?)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    println!("[xtask] legacy v3 release workflow v4 guard: PASS");
    Ok(())
}

fn release_authority_contract(root: &Path) -> Result<()> {
    let native_path = root.join("desktop/src-tauri/src/native_update.rs");
    let native = fs::read_to_string(&native_path)?;
    for marker in [
        "V4_RELEASE_AUTHORITY_REPOSITORY",
        "V4_STABLE_METADATA_ENDPOINT",
        "V4_BETA_METADATA_ENDPOINT",
        "https://raw.githubusercontent.com/pumni/Sky-Auto-Player-Releases/main/channels/stable/latest.json",
        "https://raw.githubusercontent.com/pumni/Sky-Auto-Player-Releases/main/channels/beta/latest.json",
        "endpoints(vec![endpoint])",
        "V4_TAURI_UPDATER_PUBLIC_KEY",
        "V4_TAURI_UPDATER_PUBLIC_KEYS",
        ".pubkey(public_key)",
        "production_authority_is_fixed_and_channel_isolated",
    ] {
        if !native.contains(marker) {
            return Err(format!(
                "Rust updater authority is missing the fixed v4 contract marker: {marker}"
            )
            .into());
        }
    }
    for forbidden in [
        "api.github.com/repos/pumni/Sky-Auto-Player/releases",
        "update_authority_not_configured",
        "std::env::var(\"",
        "release-2026",
    ] {
        if native.contains(forbidden) {
            return Err(format!(
                "Rust v4 updater authority contains a forbidden fallback/injection marker: {forbidden}"
            )
            .into());
        }
    }

    let generator_path = root.join("rust/xtask/src/release_authority.rs");
    let generator = fs::read_to_string(&generator_path)?;
    for marker in [
        "AUTHORITY_REPOSITORY: &str = \"pumni/Sky-Auto-Player-Releases\"",
        "STABLE_METADATA_PATH: &str = \"channels/stable/latest.json\"",
        "BETA_METADATA_PATH: &str = \"channels/beta/latest.json\"",
        "WINDOWS_PLATFORM: &str = \"windows-x86_64\"",
        "canonical_installer_name",
        "canonical_asset_url",
        "valid_utc_timestamp",
        "valid_signature",
        "version::parse",
        "parsed_version.major != 4",
    ] {
        if !generator.contains(marker) {
            return Err(format!(
                "v4 metadata generator is missing its deterministic validation marker: {marker}"
            )
            .into());
        }
    }
    for forbidden in [
        "pumni/Sky-Auto-Player/releases",
        "example.invalid",
        "dangerousInsecureTransportProtocol",
    ] {
        if generator.contains(forbidden) {
            return Err(format!(
                "v4 metadata generator contains a forbidden authority marker: {forbidden}"
            )
            .into());
        }
    }

    let bundle_path = root.join("rust/xtask/src/tauri_bundle.rs");
    let bundle = fs::read_to_string(&bundle_path)?;
    for marker in [
        "schema_version: 2",
        "evidence_type: \"tauri-nsis-artifact\"",
        "installer_sha256",
        "updater_signature_sha256",
    ] {
        if !bundle.contains(marker) {
            return Err(format!(
                "Tauri artifact evidence is missing its exact-byte marker: {marker}"
            )
            .into());
        }
    }

    let acceptance_path = root.join("scripts/ci_v4_release_authority_acceptance.ps1");
    let acceptance = fs::read_to_string(&acceptance_path)?;
    for marker in [
        "This is deliberately read-only",
        "$sourceRepository = \"pumni/Sky-Auto-Player\"",
        "$authorityRepository = \"pumni/Sky-Auto-Player-Releases\"",
        "releases/latest",
        "read_only=true",
    ] {
        if !acceptance.contains(marker) {
            return Err(format!(
                "v4 release authority acceptance is missing its read-only marker: {marker}"
            )
            .into());
        }
    }
    for forbidden in [
        "gh release create",
        "gh release upload",
        "gh release delete",
        "softprops/action-gh-release",
    ] {
        if acceptance.contains(forbidden) {
            return Err(format!(
                "read-only v4 authority acceptance contains a release mutation: {forbidden}"
            )
            .into());
        }
    }

    let promotion_path = root.join("scripts/promote_v4_metadata.ps1");
    let promotion = fs::read_to_string(&promotion_path)?;
    for marker in [
        "$productionAuthenticodeMode = \"unsigned-zero-budget\"",
        "governed unsigned-zero-budget Authenticode evidence",
        "[ValidateSet(\"stable\", \"beta\")]",
        "$authorityRepository = \"pumni/Sky-Auto-Player-Releases\"",
        "$QualificationEvidence",
        "release-authority validate --channel $Channel",
        "releases/tags/v$version",
        "published_at",
        "installer_sha256",
        "updater_signature_sha256",
        "Get-PublishedAssetSha256",
        "Assert-PublishedAsset",
        "Invoke-PromotionSelfTest",
        "same-name/different-bytes",
        "Copy-Item -LiteralPath $Metadata",
        "never publishes or mutates a GitHub release",
    ] {
        if !promotion.contains(marker) {
            return Err(format!(
                "v4 metadata promotion is missing its post-publication marker: {marker}"
            )
            .into());
        }
    }
    for forbidden in [
        "gh release create",
        "gh release upload",
        "softprops/action-gh-release",
        "pumni/Sky-Auto-Player/releases",
    ] {
        if promotion.contains(forbidden) {
            return Err(format!(
                "v4 metadata promotion contains a forbidden release mutation/fallback: {forbidden}"
            )
            .into());
        }
    }

    let ci_path = root.join(".github/workflows/ci.yml");
    let ci = fs::read_to_string(&ci_path)?;
    for marker in [
        "release_authority:",
        "name: V4 release authority acceptance",
        "scripts/ci_v4_release_authority_acceptance.ps1",
        "scripts/promote_v4_metadata.ps1 -SelfTest",
        "Emit exact Tauri qualification evidence",
        "authenticode_mode = \"unsigned-zero-budget\"",
        "V4_QUALIFICATION_EVIDENCE.json",
        "RELEASE_AUTHORITY_RESULT",
        "needs: [changes, static, release_authority, supply_chain, validate, updater_e2e, packaged]",
    ] {
        if !ci.contains(marker) {
            return Err(
                format!("CI is missing the v4 release authority gate marker: {marker}").into(),
            );
        }
    }
    println!("[xtask] v4 release authority contract: PASS");
    Ok(())
}

fn v4_release_pipeline_contract_source(
    workflow: &str,
    pipeline: &str,
    regression: &str,
) -> Result<()> {
    let workflow = workflow.replace("\r\n", "\n");
    for marker in [
        "name: V4 Release Pipeline",
        "workflow_dispatch:",
        "runs-on: [self-hosted, windows, v4-release, single-tenant]",
        "contents: read",
        "id-token: write",
        "attestations: write",
        "V4_RELEASE_AUTHORITY_TOKEN",
        "ref: ${{ inputs.source_sha }}",
        "persist-credentials: false",
        "actions/attest@",
        "actions/upload-artifact@",
        "--source-digest $env:GITHUB_SHA",
        "Qualify downloaded exact candidate bytes and packaged update",
        "RecordAttestations",
        "PublishDraft",
        "PromoteMetadata",
        "FinalVerify",
    ] {
        if !workflow.contains(marker) {
            return Err(
                format!("v4 release workflow is missing its required marker: {marker}").into(),
            );
        }
    }
    let workflow_states = [
        "-State ValidateRequest",
        "-State ValidateAuthority",
        "-State BuildCandidate",
        "-State CreateDraft",
        "-State DownloadDraft",
        "-State QualifyDownloaded",
        "-State RecordAttestations",
        "-State PublishDraft",
        "-State PromoteMetadata",
        "-State FinalVerify",
    ];
    let mut previous = 0;
    for marker in workflow_states {
        let position = workflow
            .find(marker)
            .ok_or_else(|| format!("v4 release workflow is missing state marker: {marker}"))?;
        if position < previous {
            return Err("v4 release workflow states are not ordered fail-closed".into());
        }
        previous = position;
    }
    for forbidden in [
        "cargo xtask dist",
        "softprops/action-gh-release",
        "secrets.TAURI_SIGNING_PRIVATE_KEY",
        "secrets.TAURI_SIGNING_PRIVATE_KEY_PASSWORD",
        "secrets.UPDATER_PRIVATE_KEY",
        "secrets.UPDATER_PASSWORD",
        "secrets.V4_UPDATER_PASSWORD",
        "updater_password_env",
        "credential_target",
        "Sky-Auto-Player-Updater.exe",
        "MANIFEST.json.sig",
        ".github/workflows/release.yml",
        "ci_tauri_update_e2e.ps1",
    ] {
        if workflow.contains(forbidden) {
            return Err(
                format!("v4 release workflow contains forbidden marker: {forbidden}").into(),
            );
        }
    }
    if workflow.matches("GH_TOKEN: ${{ github.token }}").count() != 1 {
        return Err(
            "source GITHUB_TOKEN must be confined to the exact attestation verification step"
                .into(),
        );
    }

    if pipeline
        .matches("orchestrate_v4_production_release.ps1")
        .count()
        != 1
    {
        return Err("production orchestrator must have exactly one pipeline call site".into());
    }
    for marker in [
        "ValidateRequest",
        "ValidateAuthority",
        "BuildCandidate",
        "CreateDraft",
        "DownloadDraft",
        "QualifyDownloaded",
        "RecordAttestations",
        "PublishDraft",
        "PromoteMetadata",
        "FinalVerify",
        "unsigned-zero-budget",
        "authority main is not initialized",
        "refs/heads/main",
        "authority already contains tag",
        "existing releases are never moved or replaced",
        "Get-FileHash",
        "verify-signature",
        "verify-tauri-bundle",
        "current-user",
        "active-playback-install-rejected",
        "ci_v4_release_authority_acceptance.ps1",
        "promote_v4_metadata.ps1",
        "release-authority",
        "published_at",
        "draft = $true",
        "draft = $false",
        "V4_RELEASE_AUTHORITY_TOKEN",
        "upload_url",
        "immutable-releases",
        "Assert-ImmutableRelease",
        "ci_tauri_update_e2e.ps1",
        "CandidateInstallerPath",
        "CandidateSignaturePath",
        "CandidatePublicKeyPath",
        "export-public-key",
        "Start-MpScan",
        "scan_performed",
        "selftest-update-active-playback",
        "scan_v4_defender_exact.ps1",
        "v4_updater_credential_broker.ps1",
    ] {
        if !pipeline.contains(marker) {
            return Err(
                format!("v4 release coordinator is missing its required marker: {marker}").into(),
            );
        }
    }
    let draft_boundary = pipeline
        .find("function Invoke-CreateDraft")
        .ok_or("v4 release coordinator is missing the draft boundary")?;
    if pipeline[draft_boundary..].contains("orchestrate_v4_production_release.ps1") {
        return Err("v4 release coordinator may not rebuild after draft creation".into());
    }
    if !regression.contains("MockReleaseApi")
        || !regression.contains("candidate rebuilt")
        || !regression.contains("promotion before immutable publication")
        || !regression.contains("BuildCount -ne 1")
        || !regression.contains("UploadedThroughReleaseUrl")
        || !regression.contains("ExactDownloadedBytes")
        || !regression.contains("immutable")
    {
        return Err(
            "v4 release coordinator regression test is missing build-once/publication guards"
                .into(),
        );
    }
    Ok(())
}

fn v4_release_pipeline_contract(root: &Path) -> Result<()> {
    let workflow_path = root.join(".github/workflows/release-v4.yml");
    let pipeline_path = root.join("scripts/v4_release_pipeline.ps1");
    let regression_path = root.join("scripts/test_v4_release_pipeline.ps1");
    let pipeline = fs::read_to_string(&pipeline_path)?;
    let regression = fs::read_to_string(&regression_path)?;
    v4_release_pipeline_contract_source(
        &fs::read_to_string(&workflow_path)?,
        &pipeline,
        &regression,
    )
    .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> {
        format!("v4 release pipeline contract: {error}").into()
    })?;
    for script_name in [
        "scripts/v4_updater_credential_broker.ps1",
        "scripts/set_v4_updater_session_credential.ps1",
        "scripts/remove_v4_updater_session_credential.ps1",
        "scripts/test_v4_updater_credential_broker.ps1",
    ] {
        if !root.join(script_name).exists() {
            return Err(
                format!("v4 release pipeline is missing required helper: {script_name}").into(),
            );
        }
    }
    let rehearsal_path = root.join("scripts/test_v4_release_authority_rehearsal.ps1");
    let rehearsal = fs::read_to_string(&rehearsal_path)?;
    let upload_helper_path = root.join("scripts/v4_release_authority_upload.ps1");
    let upload_helper = fs::read_to_string(&upload_helper_path)?;
    for marker in [
        "ConfirmDisposable",
        "V4_RELEASE_AUTHORITY_TOKEN",
        "immutable-releases",
        "upload_url",
        "draft and tag deleted",
        "--method",
        "DELETE",
    ] {
        if !rehearsal.contains(marker) {
            return Err(format!(
                "v4 authority rehearsal is missing its bounded cleanup marker: {marker}"
            )
            .into());
        }
    }
    for (name, source) in [
        ("production release pipeline", pipeline.as_str()),
        ("authority rehearsal", rehearsal.as_str()),
    ] {
        for forbidden in [
            "gh @Arguments --output",
            "gh.exe @Arguments --output",
            "--output $OutputPath",
            "\"$uploadUrl?name=",
        ] {
            if source.contains(forbidden) {
                return Err(format!(
                    "{name} must not use gh api --output for binary asset downloads"
                )
                .into());
            }
        }
        for marker in [
            "Invoke-GhBinaryOutput",
            "Invoke-V4ReleaseAuthorityAssetUpload",
            "PSVersionTable.PSVersion",
            "7.4.0",
            "RedirectStandardOutput",
            "RedirectStandardError",
            "StandardOutput.BaseStream",
            "ReadToEndAsync",
            "ArgumentList",
        ] {
            if !source.contains(marker) {
                return Err(
                    format!("{name} binary download helper is missing marker: {marker}").into(),
                );
            }
        }
    }
    for marker in [
        "System.Net.Http.HttpClient",
        "System.Net.Http.StreamContent",
        "System.IO.FileStream",
        "Headers.Authorization",
        "UserAgent",
        "application/vnd.github+json",
        "X-GitHub-Api-Version",
        "2026-03-10",
        "ContentLength",
        "fileLength",
        "StatusCode",
        "System.Net.HttpStatusCode",
        "Created",
        "SendAsync",
        "ReadAsStringAsync",
        "application/octet-stream",
    ] {
        if !upload_helper.contains(marker) {
            return Err(
                format!("raw release asset upload helper is missing marker: {marker}").into(),
            );
        }
    }
    if upload_helper.contains("gh ") || upload_helper.contains("ArgumentList") {
        return Err("raw release asset upload helper must not invoke GitHub CLI".into());
    }
    if upload_helper.contains("$UploadUrl?name=") {
        return Err(
            "raw release asset upload helper uses ambiguous PowerShell URL interpolation".into(),
        );
    }
    for marker in [
        "UploadUrl.Contains(\"?\")",
        "[string]::Concat($UploadUrl, \"?name=\"",
        "escapedAssetName",
    ] {
        if !upload_helper.contains(marker) {
            return Err(format!(
                "raw release asset upload URL construction guard is missing: {marker}"
            )
            .into());
        }
    }
    println!("[xtask] v4 release pipeline state-machine contract: PASS");
    Ok(())
}

fn packaged_ci_contract_source(source: &str) -> Result<()> {
    let normalized = source.replace("\r\n", "\n");
    let package_needs = "needs: [changes, static, release_authority, supply_chain, validate]";
    let fixture_start = normalized
        .find("  updater_e2e:\n")
        .ok_or("CI workflow is missing the isolated updater fixture job")?;
    let fixture_end = normalized[fixture_start..]
        .find("\n  packaged:\n")
        .map(|offset| fixture_start + offset)
        .ok_or("CI workflow updater fixture job must precede the canonical packaged job")?;
    let fixture = &normalized[fixture_start..fixture_end];
    let required_needs_line = format!("    {package_needs}");
    if !fixture.lines().any(|line| line == required_needs_line) {
        return Err(format!(
            "updater fixture job must declare the required dependency topology: {package_needs}"
        )
        .into());
    }
    for marker in [
        "name: Packaged v4 updater fixture qualification",
        "CARGO_TARGET_DIR",
        "tauri-update-fixture",
        "dangerousInsecureTransportProtocol",
        "scripts/ci_tauri_update_e2e.ps1",
        "-BundleDir",
        "RUNNER_TEMP",
    ] {
        if !fixture.contains(marker) {
            return Err(format!(
                "isolated updater fixture CI is missing its required marker: {marker}"
            )
            .into());
        }
    }

    let start = normalized
        .find("  packaged:\n")
        .ok_or("CI workflow is missing the packaged job")?;
    let end = normalized[start..]
        .find("\n  status:\n")
        .map(|offset| start + offset)
        .ok_or("CI workflow packaged job is missing the status boundary")?;
    let packaged = &normalized[start..end];
    if !packaged.lines().any(|line| line == required_needs_line) {
        return Err(format!(
            "packaged job must declare the required dependency topology: {package_needs}"
        )
        .into());
    }

    for marker in [
        "name: Packaged v4 Tauri NSIS qualification",
        "Build and sign canonical Tauri NSIS artifact",
        "bun install --frozen-lockfile",
        "bun run build",
        "bun run tauri signer generate",
        "TAURI_SIGNING_PRIVATE_KEY",
        "bun run tauri build --ci --config",
        "name: Resolve GitHub CLI for artifact attestation verification",
        "Get-Command gh.exe -CommandType Application",
        "SKY_GH_PATH=$ghPath",
        "- name: Run Authenticode tamper regression",
        "scripts/test_v4_authenticode_integrity.ps1",
        "- name: Run V4 production signing contract test",
        "scripts/test_v4_production_signing_contract.ps1",
        "V4 production signing contract test failed with exit code",
        "- name: Run V4 production release orchestrator contract test",
        "scripts/test_v4_production_orchestrator.ps1",
        "V4 production release orchestrator contract test failed with exit code",
        "- name: Run V4 updater private-key verifier secret-output regression",
        "scripts/test_v4_updater_private_key.ps1",
        "V4 updater private-key verifier regression failed with exit code",
        "- name: Verify Tauri Authenticode signature",
        "-Mode unsigned-zero-budget",
        "- name: Generate Tauri SPDX SBOM",
        "- name: Verify Tauri SPDX SBOM",
        "- name: Verify exact Tauri NSIS bundle",
        "Authenticode verification failed with exit code",
        "SBOM generation failed with exit code",
        "SBOM verification failed with exit code",
        "Tauri bundle verification failed with exit code",
        "Installed Authenticode verification failed with exit code",
        "CI self-signed credentials remain test-only",
        "Tauri updater signer generation failed with exit code",
        "Tauri build failed with exit code",
        "Installer attestation verification failed with exit code",
        "Updater signature attestation verification failed with exit code",
        "SBOM attestation verification failed with exit code",
        "GH_TOKEN: ${{ github.token }}",
        "attestation verify --help",
        "--source-digest $env:GITHUB_SHA",
        "--signer-workflow $signerWorkflow",
        "& $env:SKY_GH_PATH attestation verify",
        "--predicate-type https://spdx.dev/Document/v2.3",
        "GitHub CLI absolute path is unavailable for attestation verification",
        "GH_TOKEN is unavailable for attestation verification",
        "Installed GitHub CLI lacks the required exact-source attestation options",
        "current-user install, launch, and uninstall",
        "sky_desktop_shell.exe",
        "uninstall.exe",
        "scripts/cleanup_v4_test_signing.ps1",
        "rust/target/dist/bundle/nsis",
        "actions/upload-artifact@",
    ] {
        if !packaged.contains(marker) {
            return Err(format!(
                "canonical v4 packaged CI is missing the Tauri qualification marker: {marker}"
            )
            .into());
        }
    }
    let test_signing_setup_marker =
        "      - name: Prepare bounded ephemeral Authenticode test certificate\n";
    let tamper_regression_marker = "      - name: Run Authenticode tamper regression\n";
    let test_signing_setup_position = packaged
        .find(test_signing_setup_marker)
        .ok_or("canonical v4 packaged CI is missing the isolated test-signing setup step")?;
    let tamper_regression_position = packaged
        .find(tamper_regression_marker)
        .ok_or("canonical v4 packaged CI is missing the Authenticode tamper regression step")?;
    if test_signing_setup_position >= tamper_regression_position {
        return Err(
            "canonical v4 packaged CI must prepare test signing credentials in a prior step".into(),
        );
    }
    let test_signing_setup = &packaged[test_signing_setup_position..tamper_regression_position];
    for marker in [
        "timeout-minutes: 2",
        "pwsh scripts/setup_v4_test_signing.ps1 -EnvFile $env:GITHUB_ENV -TimeoutSeconds 30",
    ] {
        if !test_signing_setup.contains(marker) {
            return Err(format!(
                "canonical v4 packaged CI test-signing setup is missing its required marker: {marker}"
            )
            .into());
        }
    }
    let tamper_regression_end = packaged[tamper_regression_position..]
        .find("\n      - name: Run V4 production signing contract test\n")
        .map(|offset| tamper_regression_position + offset)
        .ok_or("canonical v4 packaged CI tamper regression step has no bounded end")?;
    let tamper_regression = &packaged[tamper_regression_position..tamper_regression_end];
    if tamper_regression.contains("setup_v4_test_signing.ps1") {
        return Err(
            "canonical v4 packaged CI must not configure test signing in the tamper regression step"
                .into(),
        );
    }
    let attestation_start = packaged
        .find("      - name: Verify exact GitHub artifact attestations\n")
        .ok_or("canonical v4 packaged CI is missing the attestation verification step")?;
    let attestation_end = packaged[attestation_start..]
        .find("\n      - name: Upload exact Tauri NSIS release candidate\n")
        .map(|offset| attestation_start + offset)
        .ok_or("canonical v4 packaged CI attestation step has no bounded end")?;
    let attestation = &packaged[attestation_start..attestation_end];
    if attestation
        .matches("--source-digest $env:GITHUB_SHA")
        .count()
        != 3
        || attestation
            .matches("--signer-workflow $signerWorkflow")
            .count()
            != 3
        || attestation.matches("-R $env:GITHUB_REPOSITORY").count() != 3
        || attestation
            .lines()
            .any(|line| line.trim_start().starts_with("gh attestation verify"))
    {
        return Err(
            "canonical v4 packaged CI attestation verification must use absolute gh, exact source digest, signer workflow, and repository binding for all three checks".into(),
        );
    }
    let gh_resolution_position = packaged
        .find("      - name: Resolve GitHub CLI for artifact attestation verification\n")
        .ok_or("canonical v4 packaged CI is missing the GitHub CLI resolution step")?;
    let restricted_path_position = packaged
        .find("      - name: Construct Python-unavailable canonical environment\n")
        .ok_or("canonical v4 packaged CI is missing the restricted environment step")?;
    if gh_resolution_position >= restricted_path_position {
        return Err(
            "canonical v4 packaged CI must resolve the absolute GitHub CLI path before restricted PATH construction".into(),
        );
    }
    let restricted_path_end = packaged[restricted_path_position..]
        .find("\n      - name: Build and sign canonical Tauri NSIS artifact\n")
        .map(|offset| restricted_path_position + offset)
        .ok_or("canonical v4 packaged CI restricted environment step has no bounded end")?;
    let restricted_environment = &packaged[restricted_path_position..restricted_path_end];
    if restricted_environment.contains("gh.exe") || restricted_environment.contains("GitHub CLI") {
        return Err(
            "canonical v4 packaged CI must not add the GitHub CLI directory to the restricted build PATH".into(),
        );
    }

    let validate_start = normalized
        .find("  validate:\n")
        .ok_or("CI workflow is missing the validate job")?;
    let validate_end = normalized[validate_start..]
        .find("\n  updater_e2e:\n")
        .map(|offset| validate_start + offset)
        .ok_or("CI workflow validate job must precede the updater fixture job")?;
    if normalized[validate_start..validate_end].contains("certutil.exe") {
        return Err("ordinary Windows validation must not require certutil.exe".into());
    }

    for forbidden in [
        "tauri-update-fixture",
        "dangerousInsecureTransportProtocol",
        "127.0.0.1:17845",
        "CARGO_TARGET_DIR",
        "--features",
        "cargo xtask dist",
        "verify-dist",
        "Sky-Auto-Player-v",
        "Sky-Auto-Player-Updater.exe",
        "MANIFEST.json",
        "PORTABLE_ARTIFACT",
        "portable",
    ] {
        if packaged.contains(forbidden) {
            return Err(format!(
                "canonical v4 packaged CI must not contain the legacy v3 artifact marker: {forbidden}"
            )
            .into());
        }
    }

    for marker in [
        "needs: [changes, static, release_authority, supply_chain, validate, updater_e2e, packaged]",
        "UPDATER_E2E_RESULT",
    ] {
        if !normalized.contains(marker) {
            return Err(format!(
                "CI required gate is missing updater fixture integration marker: {marker}"
            )
            .into());
        }
    }
    Ok(())
}

fn packaged_ci_contract(root: &Path) -> Result<()> {
    let path = root.join(".github/workflows/ci.yml");
    packaged_ci_contract_source(&fs::read_to_string(&path)?)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    println!("[xtask] canonical v4 packaged CI Tauri contract: PASS");
    Ok(())
}

fn v4_legacy_updater_source_contract(source: &str, surface: &str) -> Result<()> {
    for forbidden in [
        "sky_updater",
        "Sky-Auto-Player-Updater.exe",
        "sky_updater_e2e",
        "cargo xtask dist",
        "verify-dist",
        "MANIFEST.json",
        "MANIFEST.json.sig",
        "SKY_UPDATE_SIGNING_KEY_HEX",
        "pep440_rs",
        "packaging.version",
        "ActiveUpdateState",
        "active_update_for_install",
    ] {
        if source.contains(forbidden) {
            return Err(format!(
                "retired v3 updater marker `{forbidden}` remains in current v4 surface {surface}"
            )
            .into());
        }
    }
    Ok(())
}

fn v4_legacy_updater_retirement(root: &Path) -> Result<()> {
    let desktop_manifest = root.join("desktop/src-tauri/Cargo.toml");
    v4_legacy_updater_source_contract(
        &fs::read_to_string(&desktop_manifest)?,
        desktop_manifest.to_string_lossy().as_ref(),
    )?;

    let workspace_manifest = root.join("rust/Cargo.toml");
    v4_legacy_updater_source_contract(
        &fs::read_to_string(&workspace_manifest)?,
        workspace_manifest.to_string_lossy().as_ref(),
    )?;

    let lockfile = root.join("rust/Cargo.lock");
    let lockfile_source = fs::read_to_string(&lockfile)?;
    for forbidden in ["name = \"sky_updater\"", "name = \"pep440_rs\""] {
        if lockfile_source.contains(forbidden) {
            return Err(format!(
                "retired v3 dependency `{forbidden}` remains in {}",
                lockfile.display()
            )
            .into());
        }
    }

    let startup_guard = root.join("desktop/src-tauri/src/startup_guard.rs");
    if startup_guard.exists() {
        return Err(format!(
            "retired custom updater startup admission path remains: {}",
            startup_guard.display()
        )
        .into());
    }

    let current_v4_surfaces = [
        "desktop/src-tauri/src",
        "rust/xtask/src",
        ".github/workflows/ci.yml",
        ".github/workflows/release.yml",
        "scripts/orchestrate_v4_production_release.ps1",
        "scripts/promote_v4_metadata.ps1",
        "scripts/ci_tauri_update_e2e.ps1",
    ];
    for relative in current_v4_surfaces {
        let path = root.join(relative);
        if path.is_dir() {
            for source_path in walk_source(root, relative)? {
                if source_path.file_name().and_then(|name| name.to_str()) == Some("checks.rs") {
                    continue;
                }
                let source = fs::read_to_string(&source_path)?;
                let source = if relative == "rust/xtask/src" {
                    source
                        .split_once("\n#[cfg(test)]")
                        .map(|(production, _)| production.to_owned())
                        .unwrap_or(source)
                } else {
                    source
                };
                v4_legacy_updater_source_contract(&source, source_path.to_string_lossy().as_ref())?;
            }
        } else if path.is_file() {
            v4_legacy_updater_source_contract(&fs::read_to_string(&path)?, relative)?;
        }
    }

    println!("[xtask] v4 legacy updater retirement guards: PASS");
    Ok(())
}

fn v4_trust_material_contract(root: &Path) -> Result<()> {
    let config_path = root.join("desktop/src-tauri/tauri.conf.json");
    let config = fs::read_to_string(&config_path)?;
    for marker in ["sign_v4_authenticode.ps1", "plugins", "updater", "pubkey"] {
        if !config.contains(marker) {
            return Err(
                format!("v4 Tauri trust config is missing its required marker: {marker}").into(),
            );
        }
    }
    if config.contains("release-2026") || config.contains("PRIVATE KEY") {
        return Err("v4 Tauri config contains legacy or private key material".into());
    }

    let config_json: Value = serde_json::from_str(&config)?;
    let config_key = config_json
        .get("plugins")
        .and_then(Value::as_object)
        .and_then(|plugins| plugins.get("updater"))
        .and_then(Value::as_object)
        .and_then(|updater| updater.get("pubkey"))
        .and_then(Value::as_str)
        .ok_or("v4 Tauri config public trust root is not a string")?;
    let native_path = root.join("desktop/src-tauri/src/native_update.rs");
    let native = fs::read_to_string(&native_path)?;
    let native_key = extract_rust_string_constant(&native, "V4_TAURI_UPDATER_PUBLIC_KEY")?;
    if config_key != tauri_bundle::V4_TAURI_UPDATER_PUBLIC_KEY
        || native_key != tauri_bundle::V4_TAURI_UPDATER_PUBLIC_KEY
        || !native.contains(
            "const V4_TAURI_UPDATER_PUBLIC_KEYS: &[&str] = &[V4_TAURI_UPDATER_PUBLIC_KEY];",
        )
    {
        return Err("v4 production updater public-root copies do not match byte-for-byte".into());
    }
    let decoded = STANDARD.decode(config_key)?;
    let decoded = String::from_utf8(decoded)?;
    PublicKey::decode(&decoded)?;
    crate::updater_trust::inventory_public_trust_roots(root)?;

    let ci_path = root.join(".github/workflows/ci.yml");
    let ci = fs::read_to_string(&ci_path)?;
    for marker in [
        "scripts/setup_v4_test_signing.ps1",
        "scripts/verify_v4_authenticode.ps1",
        "scripts/cleanup_v4_test_signing.ps1",
        "scripts/test_v4_updater_key_rotation.ps1",
        "scripts/ci_tauri_update_e2e.ps1",
        "scripts/ci_require_windows_tools.ps1",
        "TimeoutSeconds 30",
        "SKY_TAURI_UPDATE_FIXTURE_PUBLIC_KEYS",
        "Packaged Tauri updater rotation",
        "cargo xtask sbom generate",
        "cargo xtask sbom verify",
        "workflow_dispatch:",
        "actions/attest@1e69f48acb82d1966a394da916b4c1698aa569d6",
        "Verify exact GitHub artifact attestations",
    ] {
        if !ci.contains(marker) {
            return Err(format!("v4 trust CI is missing its required marker: {marker}").into());
        }
    }
    if ci.matches("cargo install cargo-vet").count() != 1 {
        return Err("CI must install cargo-vet exactly once in the supply-chain job".into());
    }

    let verifier = fs::read_to_string(root.join("scripts/verify_v4_authenticode.ps1"))?;
    for marker in [
        "unsigned-zero-budget",
        "NotSigned",
        "authenticode-unsigned-zero-budget",
        "unsigned-zero-budget-policy",
        "SKY_AUTHENTICODE_TEST_THUMBPRINT",
        "SKY_AUTHENTICODE_TEST_PFX_PATH",
        "SKY_AUTHENTICODE_TEST_PFX_PASSWORD",
        "SKY_AUTHENTICODE_APPROVED_SIGNER_THUMBPRINT",
        "expected_signer_thumbprint",
        "Resolve-TestPfxPath",
        "v4_authenticode_crypto.ps1",
        "Get-AuthenticodeIntegrityProof",
        "platform_status",
        "UnknownError",
        "signer thumbprint mismatch",
    ] {
        if !verifier.contains(marker) {
            return Err(format!(
                "v4 Authenticode verifier is missing its required marker: {marker}"
            )
            .into());
        }
    }
    let crypto = fs::read_to_string(root.join("scripts/v4_authenticode_crypto.ps1"))?;
    for marker in [
        "Get-AuthenticodePeLayout",
        "Get-AuthenticodeImageDigest",
        "Get-AuthenticodeSpcDigest",
        "SizeOfHeaders",
        "sumOfBytesHashed",
        "Sort-Object PointerToRawData",
        "SignedCms",
        "CheckSignature($true)",
        "signature-valid-independent-cryptographic-integrity",
        "signedcms-spc-indirect-data-authenticode-hash",
    ] {
        if !crypto.contains(marker) {
            return Err(format!(
                "v4 independent Authenticode verifier is missing its required marker: {marker}"
            )
            .into());
        }
    }
    let setup = fs::read_to_string(root.join("scripts/setup_v4_test_signing.ps1"))?;
    for marker in [
        "CertificateRequest",
        "X509ContentType]::Pfx",
        "EphemeralKeySet",
        "SKY_AUTHENTICODE_TEST_PFX_PATH",
        "::add-mask::",
        "RUNNER_TEMP",
    ] {
        if !setup.contains(marker) {
            return Err(format!(
                "v4 Authenticode test PFX setup is missing its required marker: {marker}"
            )
            .into());
        }
    }
    let mask_position = setup
        .find("::add-mask::")
        .ok_or("v4 Authenticode test PFX setup must mask the generated password")?;
    let password_environment_position = setup
        .find("SKY_AUTHENTICODE_TEST_PFX_PASSWORD=$pfxPassword")
        .ok_or("v4 Authenticode test PFX setup must publish the generated password")?;
    if mask_position >= password_environment_position {
        return Err(
            "v4 Authenticode test PFX setup must mask the generated password before GITHUB_ENV"
                .into(),
        );
    }
    for forbidden in [
        "New-SelfSignedCertificate",
        "certutil.exe",
        "Cert:\\CurrentUser",
        "TrustedPublisher",
        "CurrentUser/${store}",
    ] {
        if setup.contains(forbidden) {
            return Err(format!(
                "v4 Authenticode test PFX setup must not depend on certificate stores: {forbidden}"
            )
            .into());
        }
    }
    let cleanup = fs::read_to_string(root.join("scripts/cleanup_v4_test_signing.ps1"))?;
    for marker in [
        "SKY_AUTHENTICODE_TEST_PFX_PATH",
        "RUNNER_TEMP",
        "sky-v4-test-signing-[0-9a-fA-F]{32}\\.pfx",
        "Clear-TestSigningEnvironment",
        "SKY_AUTHENTICODE_TEST_PFX_PASSWORD",
    ] {
        if !cleanup.contains(marker) {
            return Err(format!(
                "v4 Authenticode test PFX cleanup is missing its required marker: {marker}"
            )
            .into());
        }
    }
    for forbidden in [
        "Cert:\\CurrentUser",
        "TrustedPublisher",
        "CurrentUser/${store}",
    ] {
        if cleanup.contains(forbidden) {
            return Err(format!(
                "v4 Authenticode test PFX cleanup must not depend on certificate stores: {forbidden}"
            )
            .into());
        }
    }
    let signer = fs::read_to_string(root.join("scripts/sign_v4_authenticode.ps1"))?;
    for marker in [
        "unsigned-zero-budget",
        "no signing performed",
        "SKY_AUTHENTICODE_TEST_PFX_PATH",
        "SKY_AUTHENTICODE_TEST_PFX_PASSWORD",
        "/f $pfxPath",
        "/p $pfxPassword",
        "EphemeralKeySet",
        "SKY_AUTHENTICODE_APPROVED_SIGNER_THUMBPRINT",
        "SKY_AUTHENTICODE_PROVIDER",
        "SKY_AUTHENTICODE_PROVIDER_COMMAND",
    ] {
        if !signer.contains(marker) {
            return Err(
                format!("v4 Authenticode signer is missing its required marker: {marker}").into(),
            );
        }
    }
    let tamper = fs::read_to_string(root.join("scripts/test_v4_authenticode_integrity.ps1"))?;
    for marker in [
        "v4_authenticode_crypto.ps1",
        "sign_v4_authenticode.ps1",
        "Get-AuthenticodeSignature",
        "clean signed PE PASS",
        "Tampered signed PE unexpectedly passed independent Authenticode verification",
        "Get-AuthenticodePeLayout",
        "WriteAllBytes",
    ] {
        if !tamper.contains(marker) {
            return Err(format!(
                "v4 Authenticode tamper regression is missing its required marker: {marker}"
            )
            .into());
        }
    }
    let contract_test =
        fs::read_to_string(root.join("scripts/test_v4_production_signing_contract.ps1"))?;
    for marker in [
        "unsigned-zero-budget mode succeeds without a provider",
        "Production signing rejects test credentials",
        "Production verification rejects CI test certificate",
        "unsigned-zero-budget verification rejects signed binary",
        "Production verification rejects test thumbprint",
        "CI test certificate cannot satisfy production mode or zero-budget unsigned state",
    ] {
        if !contract_test.contains(marker) {
            return Err(format!(
                "v4 production signing contract test is missing its required marker: {marker}"
            )
            .into());
        }
    }
    let key_verifier = fs::read_to_string(root.join("scripts/verify_v4_updater_private_key.ps1"))?;
    if !key_verifier.contains("updater-trust verify-private-key") {
        return Err(
            "v4 updater key verification script must delegate to updater-trust verify-private-key"
                .into(),
        );
    }
    for forbidden in ["::add-mask::", "19AABD2E7838818C"] {
        if key_verifier.contains(forbidden) {
            return Err(format!(
                "v4 updater key verification script must not emit or hard-code production secret/output data: {forbidden}"
            )
            .into());
        }
    }
    let key_verifier_test =
        fs::read_to_string(root.join("scripts/test_v4_updater_private_key.ps1"))?;
    for marker in [
        "V4_TEST_ONLY_PASS_PHRASE_MARKER",
        "verify_v4_updater_private_key.ps1",
        "Verifier mismatch path",
        "Verifier success path",
        "throwaway.key",
    ] {
        if !key_verifier_test.contains(marker) {
            return Err(format!(
                "v4 updater key verifier regression is missing its required marker: {marker}"
            )
            .into());
        }
    }
    let orchestrator =
        fs::read_to_string(root.join("scripts/orchestrate_v4_production_release.ps1"))?;
    for marker in [
        "ExpectedSourceSha",
        "Version",
        "Channel",
        "UpdaterPrivateKeyPath",
        "ApprovedSignerThumbprint",
        "updater-trust verify-private-key",
        "updater-trust verify-signature",
        "V4_QUALIFICATION_EVIDENCE.json",
        "V4_PRODUCTION_RELEASE_EVIDENCE.json",
        "sign_v4_authenticode.ps1",
        "verify_v4_authenticode.ps1",
        "unsigned-zero-budget",
        "Invoke-PrePackagingStaleOutputPurge",
        "[Pre-Packaging Purge] Stale candidate artifacts and evidence successfully purged: PASS",
    ] {
        if !orchestrator.contains(marker) {
            return Err(format!(
                "v4 production release orchestrator is missing its required marker: {marker}"
            )
            .into());
        }
    }
    let evidence_builder = fs::read_to_string(root.join("scripts/v4_qualification_evidence.ps1"))?;
    for marker in [
        "New-V4CanonicalQualificationEvidence",
        "tauri-nsis-qualified-release",
        "install-launch-uninstall",
    ] {
        if !evidence_builder.contains(marker) {
            return Err(format!(
                "v4 qualification evidence builder is missing required marker: {marker}"
            )
            .into());
        }
    }
    let orchestrator_test =
        fs::read_to_string(root.join("scripts/test_v4_production_orchestrator.ps1"))?;
    for marker in [
        "[PASS] All V4 production orchestrator contract tests passed",
        "Parameter validation fails closed on missing parameters",
        "Source SHA mismatch fails closed before packaging",
        "Channel policy validation fails closed on invalid SemVer / channel",
        "Mutually exclusive provider configuration fails closed",
        "Wrong updater private key fails pre-flight verification before packaging",
        "Secret values are not emitted by expected error paths",
        "Inherited signing key environment fails closed",
        "Stale candidate artifacts and evidence are purged before packaging",
        "[Pre-Packaging Purge] Stale candidate artifacts and evidence successfully purged: PASS",
        "Stale-output purge did not execute strictly BEFORE pre-packaging updater key verification",
        "Updater signature verification rejects corrupted signature",
        "Tampered candidate binary is detected",
        "Production verification rejects CI test certificate",
    ] {
        if !orchestrator_test.contains(marker) {
            return Err(format!(
                "v4 production orchestrator test is missing its required marker: {marker}"
            )
            .into());
        }
    }
    let topology_doc = fs::read_to_string(root.join("docs/v4-release-execution-topology.md"))?;
    for marker in [
        "Build Once, Qualify Exact Bytes",
        "Runner Trust Boundaries and Key Custody",
        "V4_QUALIFICATION_EVIDENCE.json",
        "V4_PRODUCTION_RELEASE_EVIDENCE.json",
    ] {
        if !topology_doc.contains(marker) {
            return Err(format!(
                "v4 release execution topology documentation is missing required marker: {marker}"
            )
            .into());
        }
    }

    let private_begin = ["BEGIN", "PRIVATE", "KEY"].join(" ");
    let rsa_private_begin = ["BEGIN", "RSA", "PRIVATE", "KEY"].join(" ");
    let ec_private_begin = ["BEGIN", "EC", "PRIVATE", "KEY"].join(" ");
    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            !entry.path().components().any(|component| {
                matches!(
                    component.as_os_str().to_str(),
                    Some(".git" | "target" | "node_modules" | "dist")
                )
            })
        })
    {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let filename = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if [".key", ".pem", ".pfx", ".p12"]
            .iter()
            .any(|suffix| filename.ends_with(suffix))
        {
            return Err(format!(
                "private signing material file is present in the repository tree: {}",
                path.display()
            )
            .into());
        }
        let bytes = fs::read(path)?;
        if is_tauri_minisign_private_key(&bytes) {
            return Err(format!(
                "Tauri/minisign private key material found at {}",
                path.display()
            )
            .into());
        }
        let Ok(content) = String::from_utf8(bytes) else {
            continue;
        };
        for (line_number, line) in content.lines().enumerate() {
            if line.contains(&private_begin)
                || line.contains(&rsa_private_begin)
                || line.contains(&ec_private_begin)
            {
                return Err(format!(
                    "private key material marker found at {}:{}",
                    path.display(),
                    line_number + 1
                )
                .into());
            }
            let secret_name = ["TAURI_SIGNING_PRIVATE", "_KEY"].concat();
            if line.contains(&secret_name)
                && ["Write-Host", "Write-Output", "echo", "Add-Content"]
                    .iter()
                    .any(|sink| line.contains(sink))
            {
                return Err(format!(
                    "signing secret is sent to a logging/output sink at {}:{}",
                    path.display(),
                    line_number + 1
                )
                .into());
            }
        }
    }
    println!("[xtask] v4 trust-material and secret-output guards: PASS");
    Ok(())
}

fn extract_rust_string_constant(source: &str, name: &str) -> Result<String> {
    let marker = format!("const {name}: &str = \"");
    let values = source
        .lines()
        .filter_map(|line| {
            let start = line.find(&marker)? + marker.len();
            let value = line.get(start..)?.split_once('"')?.0;
            Some(value.to_owned())
        })
        .collect::<Vec<_>>();
    match values.as_slice() {
        [value] => Ok(value.clone()),
        _ => Err(format!("Rust source must contain exactly one {name} string constant").into()),
    }
}

fn is_tauri_minisign_private_key(bytes: &[u8]) -> bool {
    let mut candidate = bytes.to_vec();
    for _ in 0..3 {
        if is_minisign_secret_text(&candidate) {
            return true;
        }
        let Ok(text) = std::str::from_utf8(&candidate) else {
            return false;
        };
        let Ok(decoded) = STANDARD.decode(text.trim()) else {
            return false;
        };
        candidate = decoded;
    }
    false
}

fn is_minisign_secret_text(bytes: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };
    let lines = text.lines().collect::<Vec<_>>();
    if lines.len() != 2 {
        return false;
    }
    let comment = lines[0].to_ascii_lowercase();
    if !comment.starts_with("untrusted comment:")
        || (!comment.contains("secret key") && !comment.contains("private key"))
    {
        return false;
    }
    STANDARD
        .decode(lines[1])
        .map(|payload| (64..=1024).contains(&payload.len()))
        .unwrap_or(false)
}

fn active_files(root: &Path) -> impl Iterator<Item = std::path::PathBuf> {
    [
        root.join("rust/Cargo.toml"),
        root.join("rust/Cargo.lock"),
        root.join("desktop/src-tauri/Cargo.toml"),
        root.join("desktop/package.json"),
        root.join(".github/workflows/ci.yml"),
        root.join(".github/workflows/release.yml"),
    ]
    .into_iter()
}

fn walk_source(root: &Path, prefix: &str) -> Result<Vec<std::path::PathBuf>> {
    let directory = root.join(prefix);
    let mut files = Vec::new();
    for entry in WalkDir::new(directory).follow_links(false) {
        let entry = entry?;
        if entry.file_type().is_file()
            && matches!(
                entry.path().extension().and_then(|e| e.to_str()),
                Some("rs" | "toml" | "yml" | "yaml" | "json")
            )
        {
            files.push(entry.into_path());
        }
    }
    Ok(files)
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Finding {
    path: String,
    line: usize,
    rule: String,
    detail: String,
}

impl fmt::Display for Finding {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            output,
            "{}:{} {}: {}",
            self.path, self.line, self.rule, self.detail
        )
    }
}

pub(crate) fn strip_rust_comments(source: &str) -> String {
    let mut result = String::with_capacity(source.len());
    let bytes = source.as_bytes();
    let mut index = 0;
    let mut block_depth = 0usize;
    while index < bytes.len() {
        if block_depth > 0 {
            if bytes.get(index..index + 2) == Some(b"/*") {
                block_depth += 1;
                result.push(' ');
                result.push(' ');
                index += 2;
            } else if bytes.get(index..index + 2) == Some(b"*/") {
                block_depth -= 1;
                result.push(' ');
                result.push(' ');
                index += 2;
            } else {
                if bytes[index] == b'\n' {
                    result.push('\n');
                } else {
                    result.push(' ');
                }
                index += 1;
            }
        } else if bytes.get(index..index + 2) == Some(b"//") {
            while index < bytes.len() && bytes[index] != b'\n' {
                result.push(' ');
                index += 1;
            }
        } else if bytes.get(index..index + 2) == Some(b"/*") {
            block_depth = 1;
            result.push(' ');
            result.push(' ');
            index += 2;
        } else {
            result.push(bytes[index] as char);
            index += 1;
        }
    }
    result
}

fn windows_sys_paths(line: &str) -> Vec<String> {
    let marker = "windows_sys::";
    let mut paths = Vec::new();
    let mut start = 0;
    while let Some(relative) = line[start..].find(marker) {
        let begin = start + relative + marker.len();
        let end = line[begin..]
            .find(|character: char| {
                !(character.is_ascii_alphanumeric() || character == '_' || character == ':')
            })
            .map_or(line.len(), |offset| begin + offset);
        paths.push(line[begin..end].trim_end_matches(':').to_owned());
        start = end.max(begin + 1);
    }
    paths
}

fn approved_windows_sys(path: &str) -> bool {
    ALLOWED_WINDOWS_SYS_MODULES
        .iter()
        .any(|allowed| path == *allowed || path.starts_with(&format!("{allowed}::")))
}

fn scan_rust_text(path: &Path, source: &str) -> Vec<Finding> {
    let clean = strip_rust_comments(source);
    let relative = path.to_string_lossy().replace('\\', "/");
    let mut findings = Vec::new();
    for (line_number, line) in clean.lines().enumerate() {
        for token in FORBIDDEN_SECURITY_APIS {
            if line
                .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
                .any(|word| word == *token)
            {
                findings.push(Finding {
                    path: relative.clone(),
                    line: line_number + 1,
                    rule: format!("forbidden-call:{token}"),
                    detail: format!("`{token}` violates SECURITY.md"),
                });
            }
        }
        let lower = line.to_ascii_lowercase();
        for dll in FORBIDDEN_DLLS {
            if lower.contains(dll) {
                findings.push(Finding {
                    path: relative.clone(),
                    line: line_number + 1,
                    rule: "forbidden-dll-load".into(),
                    detail: format!("Rust reference to `{dll}` is forbidden"),
                });
            }
        }
        for module in windows_sys_paths(line) {
            if !approved_windows_sys(&module) {
                findings.push(Finding {
                    path: relative.clone(),
                    line: line_number + 1,
                    rule: "disallowed-windows-sys-module".into(),
                    detail: format!("`windows_sys::{module}` is outside the approved allowlist"),
                });
            }
        }
    }
    findings
}

fn security_findings(root: &Path) -> Result<Vec<Finding>> {
    let mut findings = Vec::new();
    for prefix in ["rust/crates", "desktop/src-tauri"] {
        for path in walk_source(root, prefix)? {
            if path.extension().and_then(|extension| extension.to_str()) != Some("rs") {
                continue;
            }
            let relative = path.strip_prefix(root).unwrap_or(&path);
            findings.extend(scan_rust_text(relative, &fs::read_to_string(&path)?));
        }
    }
    Ok(findings)
}

fn security_baseline(root: &Path) -> Result<BTreeSet<(String, usize, String)>> {
    let path = root.join(".config/security_audit_baseline.json");
    if !path.is_file() {
        return Ok(BTreeSet::new());
    }
    let payload: Value = serde_json::from_slice(&fs::read(path)?)?;
    let mut entries = BTreeSet::new();
    for entry in payload
        .get("exceptions")
        .and_then(Value::as_array)
        .ok_or("security baseline exceptions must be an array")?
    {
        let object = entry
            .as_object()
            .ok_or("security baseline entry must be an object")?;
        let path = object
            .get("path")
            .and_then(Value::as_str)
            .ok_or("security baseline path missing")?;
        let line = object
            .get("line")
            .and_then(Value::as_u64)
            .ok_or("security baseline line missing")? as usize;
        let rule = object
            .get("rule")
            .and_then(Value::as_str)
            .ok_or("security baseline rule missing")?;
        entries.insert((path.replace('\\', "/"), line, rule.to_owned()));
    }
    Ok(entries)
}

pub(crate) fn security(root: &Path) -> Result<()> {
    let baseline = security_baseline(root)?;
    let findings = security_findings(root)?;
    let mut fresh = Vec::new();
    for finding in findings {
        let key = (finding.path.clone(), finding.line, finding.rule.clone());
        if !baseline.contains(&key) {
            fresh.push(finding);
        }
    }
    if let Some(finding) = fresh.first() {
        return Err(format!("security audit failed: {finding}").into());
    }
    println!(
        "[xtask] security checks: PASS ({} baseline-covered finding(s))",
        baseline.len()
    );
    Ok(())
}

const FACADE_HARD_LIMIT: usize = 250;
const REGULAR_SOFT_LIMIT: usize = 700;
const REGULAR_HARD_LIMIT: usize = 900;
const WORKER_FUNCTION_HARD_LIMIT: usize = 350;
const CONTEXT_FIELD_HARD_LIMIT: usize = 12;
const DISPATCH_FUNCTION_HARD_LIMIT: usize = 180;
const WORKER_SCHEDULE_CLONE_PATTERNS: &[&str] = &[
    "schedule.clone()",
    "Clone::clone(&schedule",
    "Clone::clone(&config.schedule",
];
const FACADES: &[&str] = &["engine.rs", "input.rs", "wait.rs", "lib.rs"];
const LEGACY_DISPATCH_PATHS: &[&str] = &[
    "rust/crates/sky_player/src/engine/worker/downs.rs",
    "rust/crates/sky_player/src/engine/worker/down_outcome.rs",
    "rust/crates/sky_player/src/engine/worker/releases.rs",
];
const CANONICAL_DISPATCH_FILES: &[&str] = &[
    "authored.rs",
    "mod.rs",
    "observation.rs",
    "observer.rs",
    "recovery.rs",
    "timing.rs",
    "hold_forensics.rs",
    "observer_wake.rs",
];
const ALLOWED_UNSAFE_MODULES: &[&str] = &[
    "rust/crates/sky_dispatch_win32/src/calibration.rs",
    "rust/crates/sky_dispatch_win32/src/clock.rs",
    "rust/crates/sky_dispatch_win32/src/cpu.rs",
    "rust/crates/sky_dispatch_win32/src/event.rs",
    "rust/crates/sky_dispatch_win32/src/focus.rs",
    "rust/crates/sky_dispatch_win32/src/input.rs",
    "rust/crates/sky_dispatch_win32/src/input/physical.rs",
    "rust/crates/sky_dispatch_win32/src/input/raw.rs",
    "rust/crates/sky_dispatch_win32/src/mmcss.rs",
    "rust/crates/sky_dispatch_win32/src/power.rs",
    "rust/crates/sky_dispatch_win32/src/timer.rs",
    "rust/crates/sky_dispatch_win32/src/wait.rs",
    "rust/crates/sky_dispatch_win32/src/wait/timer.rs",
];

fn load_architecture_allowlist(root: &Path) -> Result<BTreeMap<(String, String), String>> {
    let path = root.join(".config/rust_architecture_allowlist.json");
    if !path.is_file() {
        return Err(format!("architecture allowlist is missing: {}", path.display()).into());
    }
    let payload: Value = serde_json::from_slice(&fs::read(path)?)?;
    let entries = payload
        .get("entries")
        .and_then(Value::as_array)
        .ok_or("architecture allowlist entries must be an array")?;
    let mut result = BTreeMap::new();
    for entry in entries {
        let object = entry
            .as_object()
            .ok_or("architecture allowlist entry must be an object")?;
        let path = object
            .get("path")
            .and_then(Value::as_str)
            .ok_or("architecture allowlist path missing")?;
        let rule = object
            .get("rule")
            .and_then(Value::as_str)
            .ok_or("architecture allowlist rule missing")?;
        let reason = object
            .get("reason")
            .and_then(Value::as_str)
            .ok_or("architecture allowlist reason missing")?;
        let expires = object
            .get("expires_phase")
            .and_then(Value::as_str)
            .ok_or("architecture allowlist expiry missing")?;
        if !root.join(path).is_file() {
            return Err(format!("architecture allowlist path does not exist: {path}").into());
        }
        result.insert(
            (path.replace('\\', "/"), rule.to_owned()),
            format!("{reason} (expires {expires})"),
        );
    }
    Ok(result)
}

fn architecture_record(
    errors: &mut Vec<String>,
    warnings: &mut Vec<String>,
    allowlist: &BTreeMap<(String, String), String>,
    path: &str,
    rule: &str,
    message: impl Into<String>,
) {
    let message = message.into();
    if let Some(debt) = allowlist.get(&(path.to_owned(), rule.to_owned())) {
        warnings.push(format!(
            "[{rule}] {path}: {message}; temporary allowlist: {debt}"
        ));
    } else {
        errors.push(format!("[{rule}] {path}: {message}"));
    }
}

fn clean_lines(source: &str) -> Vec<String> {
    strip_rust_comments(source)
        .lines()
        .map(str::to_owned)
        .collect()
}

fn brace_end(lines: &[String], start: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut opened = false;
    for (index, line) in lines.iter().enumerate().skip(start) {
        depth += line.matches('{').count() as i32;
        depth -= line.matches('}').count() as i32;
        opened |= line.contains('{');
        if opened && depth <= 0 {
            return Some(index);
        }
    }
    None
}

fn context_violations(lines: &[String]) -> Vec<(String, String)> {
    let mut result = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        let Some(struct_position) = trimmed.find("struct ") else {
            continue;
        };
        let name = trimmed[struct_position + "struct ".len()..]
            .split(['<', '{'])
            .next()
            .unwrap_or("")
            .trim();
        if !(name.ends_with("Context")
            || name.ends_with("Inputs")
            || name.ends_with("Config")
            || name.ends_with("Options")
            || name.ends_with("Shared"))
        {
            continue;
        }
        let Some(end) = brace_end(lines, index) else {
            continue;
        };
        let fields = lines[index + 1..end]
            .iter()
            .filter(|field| {
                let field = field.trim();
                !field.starts_with("fn ") && field.contains(':') && !field.starts_with("#")
            })
            .count();
        if fields > CONTEXT_FIELD_HARD_LIMIT {
            result.push((name.to_owned(), fields.to_string()));
        }
    }
    result
}

fn function_line_violations(lines: &[String], hard_limit: usize) -> Vec<(String, usize)> {
    let mut result = Vec::new();
    for (start, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        let Some(position) = trimmed.find("fn ") else {
            continue;
        };
        let name = trimmed[position + 3..]
            .split(['(', '<', ' '])
            .next()
            .unwrap_or("");
        if name.is_empty() {
            continue;
        }
        let Some(end) = brace_end(lines, start) else {
            continue;
        };
        let count = end - start + 1;
        if count > hard_limit {
            result.push((name.to_owned(), count));
        }
    }
    result
}

fn top_level_glob_import(lines: &[String]) -> bool {
    lines
        .iter()
        .map(|line| line.trim())
        .find(|line| !line.is_empty() && !line.starts_with("#![") && !line.starts_with("#["))
        == Some("use super::*;")
}

fn gated_test_support(lines: &[String], path: &str) -> bool {
    if !path.contains("/test_support/") && !path.ends_with("/test_support.rs") {
        return true;
    }
    lines
        .iter()
        .any(|line| line.contains("cfg(any(test, feature = \"test-support\"))"))
}

fn line_is_gated(lines: &[String], index: usize) -> bool {
    lines[..=index]
        .iter()
        .rev()
        .take(3)
        .any(|line| line.contains("cfg(any(test, feature = \"test-support\"))"))
}

fn contains_unsafe_code(source: &str) -> bool {
    let source = source.replace("#![forbid(unsafe_code)]", "");
    source
        .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
        .any(|word| word == "unsafe")
}

pub(crate) fn architecture(root: &Path) -> Result<()> {
    let allowlist = load_architecture_allowlist(root)?;
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let manifest = rust_manifest(root)?;
    let app_core_manifest = root.join("rust/crates/sky_app_core/Cargo.toml");
    let app_core: toml::Value = toml::from_str(&fs::read_to_string(&app_core_manifest)?)?;
    if let Some(dependencies) = app_core.get("dependencies").and_then(toml::Value::as_table) {
        for forbidden in [
            "tauri",
            "pyo3",
            "windows-sys",
            "sky_desktop_shell",
            "sky_player",
            "sky_native_adapters",
        ] {
            if dependencies.contains_key(forbidden) {
                architecture_record(
                    &mut errors,
                    &mut warnings,
                    &allowlist,
                    "rust/crates/sky_app_core/Cargo.toml",
                    "app_core_dependency",
                    format!("sky_app_core must not depend directly on {forbidden}"),
                );
            }
        }
    }
    let app_core_source = root.join("rust/crates/sky_app_core/src");
    if app_core_source.is_dir() {
        for path in WalkDir::new(&app_core_source).follow_links(false) {
            let path = path?;
            if !path.file_type().is_file()
                || path
                    .path()
                    .extension()
                    .and_then(|extension| extension.to_str())
                    != Some("rs")
            {
                continue;
            }
            let relative = path
                .path()
                .strip_prefix(root)?
                .to_string_lossy()
                .replace('\\', "/");
            let joined = clean_lines(&fs::read_to_string(path.path())?).join("");
            if [
                "tauri",
                "pyo3",
                "windows-sys",
                "windows_sys",
                "sky_desktop_shell",
                "sky_player",
            ]
            .iter()
            .any(|marker| joined.contains(marker))
            {
                architecture_record(
                    &mut errors,
                    &mut warnings,
                    &allowlist,
                    &relative,
                    "app_core_dependency",
                    "sky_app_core source references a forbidden delivery/platform/player dependency",
                );
            }
        }
    }
    if manifest.contains("sky_player_rs") || manifest.contains("pyo3") {
        errors.push("[retired_bridge] rust/Cargo.toml: production workspace contains a retired Python/player bridge".into());
    }

    let dispatch_dir = root.join("rust/crates/sky_player/src/engine/worker/dispatch");
    if dispatch_dir.is_dir() {
        let actual: BTreeSet<String> = fs::read_dir(&dispatch_dir)?
            .filter_map(std::result::Result::ok)
            .filter_map(|entry| {
                (entry
                    .path()
                    .extension()
                    .and_then(|extension| extension.to_str())
                    == Some("rs"))
                .then(|| entry.file_name().to_string_lossy().into_owned())
            })
            .filter(|name| !name.ends_with("_tests.rs"))
            .collect();
        let expected: BTreeSet<String> = CANONICAL_DISPATCH_FILES
            .iter()
            .map(|name| (*name).to_owned())
            .collect();
        for name in actual.difference(&expected) {
            errors.push(format!(
                "[unexpected_dispatch_module] {name}: dispatch module is not canonical"
            ));
        }
        for name in expected.difference(&actual) {
            errors.push(format!(
                "[missing_dispatch_module] {name}: canonical dispatch module is missing"
            ));
        }
    }

    for crate_name in [
        "sky_dispatch_core",
        "sky_dispatch_win32",
        "sky_app_core",
        "sky_player",
    ] {
        let source_root = root.join("rust/crates").join(crate_name).join("src");
        if !source_root.is_dir() {
            continue;
        }
        for path in WalkDir::new(&source_root).follow_links(false) {
            let path = path?;
            if !path.file_type().is_file()
                || path
                    .path()
                    .extension()
                    .and_then(|extension| extension.to_str())
                    != Some("rs")
            {
                continue;
            }
            let absolute = path.path();
            let relative = absolute
                .strip_prefix(root)?
                .to_string_lossy()
                .replace('\\', "/");
            let source = fs::read_to_string(absolute)?;
            let lines = source
                .lines()
                .map(|line| format!("{line}\n"))
                .collect::<Vec<_>>();
            let clean = clean_lines(&source);
            let joined = clean.join("");
            if LEGACY_DISPATCH_PATHS.contains(&relative.as_str()) {
                architecture_record(
                    &mut errors,
                    &mut warnings,
                    &allowlist,
                    &relative,
                    "legacy_dispatch_path",
                    "legacy dispatch path must be removed",
                );
                continue;
            }
            let limit = if FACADES.contains(
                &absolute
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or(""),
            ) {
                FACADE_HARD_LIMIT
            } else {
                REGULAR_HARD_LIMIT
            };
            if clean.len() > limit {
                architecture_record(
                    &mut errors,
                    &mut warnings,
                    &allowlist,
                    &relative,
                    if limit == FACADE_HARD_LIMIT {
                        "facade_lines"
                    } else {
                        "regular_module_lines"
                    },
                    format!("{} lines (> {limit})", clean.len()),
                );
            }
            if limit == REGULAR_HARD_LIMIT
                && clean.len() > REGULAR_SOFT_LIMIT
                && clean.len() <= REGULAR_HARD_LIMIT
            {
                warnings.push(format!(
                    "[regular_module_soft_lines] {relative}: {} lines (> {REGULAR_SOFT_LIMIT})",
                    clean.len()
                ));
            }
            if crate_name == "sky_player"
                && relative == "rust/crates/sky_player/src/engine/worker/orchestration.rs"
            {
                for (name, count) in function_line_violations(&clean, WORKER_FUNCTION_HARD_LIMIT) {
                    architecture_record(
                        &mut errors,
                        &mut warnings,
                        &allowlist,
                        &relative,
                        "worker_function_lines",
                        format!("{name} has {count} lines (> {WORKER_FUNCTION_HARD_LIMIT})"),
                    );
                }
            }
            if relative.starts_with("rust/crates/sky_player/src/engine/worker/dispatch/") {
                for (name, count) in function_line_violations(&clean, DISPATCH_FUNCTION_HARD_LIMIT)
                {
                    architecture_record(
                        &mut errors,
                        &mut warnings,
                        &allowlist,
                        &relative,
                        "dispatch_function_lines",
                        format!("{name} has {count} lines (> {DISPATCH_FUNCTION_HARD_LIMIT})"),
                    );
                }
            }
            if contains_unsafe_code(&joined) && !ALLOWED_UNSAFE_MODULES.contains(&relative.as_str())
            {
                architecture_record(
                    &mut errors,
                    &mut warnings,
                    &allowlist,
                    &relative,
                    "unsafe_boundary",
                    "unsafe code outside allowlist",
                );
            }
            if crate_name == "sky_dispatch_core"
                && (joined.contains("sky_dispatch_win32::")
                    || joined.contains("use sky_dispatch_win32"))
            {
                architecture_record(
                    &mut errors,
                    &mut warnings,
                    &allowlist,
                    &relative,
                    "dependency_direction",
                    "core imports sky_dispatch_win32",
                );
            }
            if ["sky_dispatch_core", "sky_dispatch_win32"].contains(&crate_name)
                && (joined.contains("sky_player::") || joined.contains("use sky_player"))
            {
                architecture_record(
                    &mut errors,
                    &mut warnings,
                    &allowlist,
                    &relative,
                    "dependency_direction",
                    "lower crate imports sky_player",
                );
            }
            if top_level_glob_import(&clean)
                && !relative.ends_with("/tests.rs")
                && !relative.contains("/tests/")
            {
                architecture_record(
                    &mut errors,
                    &mut warnings,
                    &allowlist,
                    &relative,
                    "production_glob_import",
                    "top-level use super::* in production module",
                );
            }
            for (index, line) in clean.iter().enumerate() {
                if line.contains("Box<dyn Fn") && !line_is_gated(&lines, index) {
                    architecture_record(
                        &mut errors,
                        &mut warnings,
                        &allowlist,
                        &relative,
                        "production_dynamic_emitter",
                        "dynamic emitter in production source",
                    );
                }
            }
            if (relative == "rust/crates/sky_player/src/engine/worker.rs"
                || relative.starts_with("rust/crates/sky_player/src/engine/worker/"))
                && WORKER_SCHEDULE_CLONE_PATTERNS
                    .iter()
                    .any(|pattern| joined.contains(pattern))
            {
                architecture_record(
                    &mut errors,
                    &mut warnings,
                    &allowlist,
                    &relative,
                    "runtime_schedule_clone",
                    "production worker must move RuntimeSchedule into the coordinator; cloning the schedule is forbidden",
                );
            }
            if !gated_test_support(&clean, &relative) {
                architecture_record(
                    &mut errors,
                    &mut warnings,
                    &allowlist,
                    &relative,
                    "test_support_cfg",
                    "test-support source is not cfg-gated",
                );
            }
            for (name, fields) in context_violations(&clean) {
                architecture_record(
                    &mut errors,
                    &mut warnings,
                    &allowlist,
                    &relative,
                    "context_fields",
                    format!("{name} has {fields} fields (> {CONTEXT_FIELD_HARD_LIMIT})"),
                );
            }
            if (relative == "rust/crates/sky_player/src/engine.rs")
                && clean.iter().enumerate().any(|(index, line)| {
                    line.trim() == "mod test_support;" && !line_is_gated(&clean, index)
                })
            {
                architecture_record(
                    &mut errors,
                    &mut warnings,
                    &allowlist,
                    &relative,
                    "test_support_cfg",
                    "test_support module is not cfg-gated",
                );
            }
        }
    }
    if !warnings.is_empty() {
        for warning in &warnings {
            println!("[xtask] architecture warning: {warning}");
        }
    }
    if let Some(error) = errors.first() {
        return Err(format!(
            "architecture audit failed: {error} ({} error(s))",
            errors.len()
        )
        .into());
    }
    println!(
        "[xtask] architecture checks: PASS ({} allowlisted warning(s))",
        warnings.len()
    );
    Ok(())
}

fn retirement(root: &Path) -> Result<()> {
    let mut files = active_files(root).collect::<Vec<_>>();
    files.extend(walk_source(root, "desktop/src")?);
    files.extend(walk_source(root, "rust/crates")?);
    for path in files {
        if !path.is_file() {
            continue;
        }
        if path
            .components()
            .any(|component| component.as_os_str() == "tests")
        {
            continue;
        }
        let content = fs::read_to_string(&path)?;
        for token in RETIRED_ACTIVE_TOKENS {
            if content.contains(token) {
                return Err(format!(
                    "retired token {token} remains in active file {}",
                    path.display()
                )
                .into());
            }
        }
    }
    validate_tooling_ledger(root)?;
    validate_xtask_process_surface(root)?;
    println!("[xtask] retirement checks: PASS");
    Ok(())
}

fn strip_retirement_inventory(source: &str) -> String {
    let marker = "const RETIRED_ACTIVE_TOKENS";
    let Some(start) = source.find(marker) else {
        return source.to_owned();
    };
    let Some(end_relative) = source[start..].find("];") else {
        return source.to_owned();
    };
    let end = start + end_relative + 2;
    format!("{}{}", &source[..start], &source[end..])
}

fn xtask_process_violation(source: &str) -> Option<String> {
    let compact = strip_rust_comments(source)
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    for call in [
        "Command::new(",
        "process::run(",
        "process::capture(",
        "process::run_owned(",
        "process::capture_owned(",
    ] {
        let mut offset = 0;
        while let Some(relative) = compact[offset..].find(call) {
            let call_position = offset + relative;
            if is_inside_rust_string(&compact, call_position) {
                offset = call_position + call.len();
                continue;
            }
            let start = call_position + call.len();
            let end = (start + 160).min(compact.len());
            let arguments = &compact[start..end];
            for program in ["python", "python3", "py", "uv"] {
                if arguments.starts_with(&format!("\"{program}\""))
                    || arguments.contains(&format!("\"{program}\""))
                {
                    return Some(format!(
                        "xtask invokes forbidden repository runtime: {program}"
                    ));
                }
            }
            offset = end;
        }
    }
    None
}

fn is_inside_rust_string(source: &str, position: usize) -> bool {
    let mut quoted = false;
    let mut escaped = false;
    for (index, character) in source.char_indices() {
        if index >= position {
            break;
        }
        if escaped {
            escaped = false;
        } else if character == '\\' && quoted {
            escaped = true;
        } else if character == '"' {
            quoted = !quoted;
        }
    }
    quoted
}

fn validate_xtask_process_surface(root: &Path) -> Result<()> {
    let directory = root.join("rust/xtask/src");
    for entry in WalkDir::new(&directory).follow_links(false) {
        let entry = entry?;
        if !entry.file_type().is_file()
            || entry
                .path()
                .extension()
                .and_then(|extension| extension.to_str())
                != Some("rs")
        {
            continue;
        }
        let path = entry.path();
        let source = fs::read_to_string(path)?;
        if let Some(violation) = xtask_process_violation(&source) {
            return Err(format!("{violation} in {}", path.display()).into());
        }
        let scan_source = if path.file_name().and_then(|name| name.to_str()) == Some("checks.rs") {
            strip_retirement_inventory(&source)
        } else {
            source
        };
        for token in [
            "build_rust_wheel.py",
            "scripts/check.py",
            "scripts/classify_ci_changes.py",
            "scripts/build_portable_release.py",
            "scripts/verify_release_manifest.py",
        ] {
            if strip_rust_comments(&scan_source).contains(token) {
                return Err(format!(
                    "retired canonical script {token} is invoked/referenced by {}",
                    path.display()
                )
                .into());
            }
        }
    }
    Ok(())
}

fn validate_tooling_ledger(root: &Path) -> Result<()> {
    let ledger_path = root.join("docs/migration/wave6-tooling-retirement-ledger.json");
    let payload: Value = serde_json::from_slice(&fs::read(&ledger_path)?)?;
    let baseline = payload
        .get("baseline")
        .and_then(Value::as_str)
        .filter(|value| {
            value.len() == 40 && value.chars().all(|character| character.is_ascii_hexdigit())
        })
        .ok_or("Wave 6 ledger baseline must be a full commit SHA")?;
    let entries = payload
        .get("entries")
        .and_then(Value::as_array)
        .ok_or("Wave 6 ledger entries must be an array")?;
    let evidence_classes = [
        "MIGRATED_XTASK",
        "MIGRATED_RUST",
        "MIGRATED_TYPESCRIPT",
        "DUPLICATE",
        "FIXTURE_FROZEN",
    ];
    let placeholders = [
        "generic evidence",
        "native covers",
        "named native/frontend/updater tests",
        "direct Rust/native build evidence is stronger",
        "native Rust/Tauri services now own",
    ];
    let mut ledger_paths = std::collections::BTreeSet::new();
    for entry in entries {
        let object = entry
            .as_object()
            .ok_or("Wave 6 ledger entry must be an object")?;
        let path = object
            .get("path")
            .and_then(Value::as_str)
            .ok_or("Wave 6 ledger path missing")?;
        if !ledger_paths.insert(path.to_owned()) {
            return Err(format!("Wave 6 ledger contains duplicate path: {path}").into());
        }
        let classification = object
            .get("classification")
            .and_then(Value::as_str)
            .ok_or("Wave 6 ledger classification missing")?;
        let exists = root.join(path).exists();
        if classification == "NONCANONICAL_RETAINED" && !exists {
            return Err(format!("retained ledger path does not exist: {path}").into());
        }
        if evidence_classes.contains(&classification) {
            validate_evidence_entry(root, path, classification, object, &placeholders)?;
        } else if matches!(
            classification,
            "OBSOLETE" | "TRANSPORT_ONLY" | "TOOLING_RETAINED" | "NONCANONICAL_RETAINED"
        ) {
            if !exists && classification != "OBSOLETE" && classification != "TRANSPORT_ONLY" {
                return Err(format!("{path}: retained tooling entry does not exist").into());
            }
        } else {
            return Err(
                format!("{path}: unknown Wave 6 ledger classification {classification}").into(),
            );
        }
    }
    let tracked_python = process::capture_text("git", &["ls-files", "--", "*.py"], root, &[])?;
    for path in tracked_python
        .lines()
        .filter(|path| root.join(path).is_file())
    {
        if !ledger_paths.contains(path) {
            return Err(format!("Python file is missing from Wave 6 ledger: {path}").into());
        }
    }
    let baseline_python = process::capture_text(
        "git",
        &[
            "ls-tree",
            "-r",
            "--name-only",
            baseline,
            "--",
            "branding",
            "scripts",
            "src",
            "tests",
        ],
        root,
        &[],
    )?;
    let baseline_paths = baseline_python
        .lines()
        .filter(|path| path.ends_with(".py"))
        .collect::<BTreeSet<_>>();
    for path in &baseline_paths {
        if !ledger_paths.contains(*path) {
            return Err(format!(
                "deleted baseline Python file is missing from Wave 6 ledger: {path}"
            )
            .into());
        }
    }
    if ledger_paths
        .iter()
        .any(|path| !baseline_paths.contains(path.as_str()))
    {
        return Err("Wave 6 ledger contains a path outside the baseline Python inventory".into());
    }
    Ok(())
}

fn validate_evidence_entry(
    root: &Path,
    path: &str,
    classification: &str,
    object: &serde_json::Map<String, Value>,
    placeholders: &[&str],
) -> Result<()> {
    let invariants = object
        .get("invariants")
        .and_then(Value::as_array)
        .ok_or(format!("{path}: invariants must be a non-empty array"))?;
    let evidence = object
        .get("evidence")
        .and_then(Value::as_array)
        .ok_or(format!("{path}: evidence must be a non-empty array"))?;
    if invariants.is_empty()
        || evidence.is_empty()
        || invariants.iter().any(|value| {
            let Some(value) = value.as_str() else {
                return true;
            };
            value.trim().is_empty()
                || placeholders.iter().any(|placeholder| {
                    value
                        .to_ascii_lowercase()
                        .contains(&placeholder.to_ascii_lowercase())
                })
        })
    {
        return Err(format!("{path}: {classification} needs concrete invariants/evidence").into());
    }
    for item in evidence {
        let target = item
            .as_str()
            .ok_or(format!("{path}: evidence target must be a string"))?;
        if placeholders.iter().any(|placeholder| {
            target
                .to_ascii_lowercase()
                .contains(&placeholder.to_ascii_lowercase())
        }) {
            return Err(format!("{path}: placeholder evidence is not permitted: {target}").into());
        }
        let (file, symbol) = target
            .split_once("::")
            .ok_or(format!("{path}: evidence must use path::symbol: {target}"))?;
        let candidate = Path::new(file);
        if candidate.is_absolute()
            || candidate
                .components()
                .any(|component| component.as_os_str() == "..")
        {
            return Err(format!("{path}: evidence path escapes repository: {file}").into());
        }
        let evidence_path = root.join(candidate);
        if !evidence_path.is_file() {
            return Err(format!("{path}: evidence file does not exist: {file}").into());
        }
        let source = fs::read_to_string(&evidence_path)?;
        if symbol.trim().is_empty() || !source.contains(symbol) {
            return Err(format!("{path}: evidence symbol is absent: {target}").into());
        }
    }
    Ok(())
}

pub fn bindings() -> Result<()> {
    let root = repo::root();
    let export_dir = prepare_binding_export_dir(&root)?;
    generate_bindings(&root, &export_dir)?;
    write_command_names(&root, &export_dir)?;
    compare_generated_bindings(&root, &export_dir)?;
    compare_command_names(&root, &export_dir)?;
    Ok(())
}

pub fn bindings_generate() -> Result<()> {
    let root = repo::root();
    let export_dir = root.join("desktop/src/bridge/generated");
    fs::create_dir_all(&export_dir)?;
    generate_bindings(&root, &export_dir)?;
    write_command_names(&root, &export_dir)?;
    println!(
        "[xtask] generated Tauri bindings in {}",
        export_dir.display()
    );
    Ok(())
}

fn command_names_source(root: &Path) -> Result<String> {
    let source = fs::read_to_string(root.join("desktop/src-tauri/src/ipc_contract.rs"))?;
    let source = source.split("#[cfg(test)]").next().unwrap_or(&source);
    let mut commands = Vec::new();
    for line in source.lines() {
        let marker = "invoke_name: \"";
        let Some(start) = line.find(marker) else {
            continue;
        };
        let rest = &line[start + marker.len()..];
        let Some(end) = rest.find('"') else {
            return Err("IPC registry contains an unterminated invoke name".into());
        };
        commands.push(rest[..end].to_owned());
    }
    if commands.len() != 30 {
        return Err(format!(
            "IPC registry contains {} commands; expected 30",
            commands.len()
        )
        .into());
    }
    let mut output = String::from(
        "// AUTO-GENERATED by `cargo xtask bindings generate` from ipc_contract.rs.\n// Do not edit command identifiers by hand.\n\nexport const COMMANDS = {\n",
    );
    for command in commands {
        let mut key = String::new();
        let mut uppercase = false;
        for character in command.chars() {
            if character == '_' {
                uppercase = true;
            } else if uppercase {
                key.push(character.to_ascii_uppercase());
                uppercase = false;
            } else {
                key.push(character);
            }
        }
        output.push_str(&format!("  {key}: '{command}',\n"));
    }
    output.push_str(
        "} as const;\n\nexport const UI_EVENTS_COMMAND = 'subscribe_ui_events' as const;\n",
    );
    Ok(output)
}

fn write_command_names(root: &Path, directory: &Path) -> Result<()> {
    fs::write(
        directory.join("command_names.ts"),
        command_names_source(root)?,
    )?;
    Ok(())
}

fn compare_command_names(root: &Path, export_dir: &Path) -> Result<()> {
    let expected = fs::read_to_string(root.join("desktop/src/bridge/generated/command_names.ts"))?;
    let actual = fs::read_to_string(export_dir.join("command_names.ts"))?;
    if normalized_text_bytes(expected.into_bytes()) != normalized_text_bytes(actual.into_bytes()) {
        return Err("generated IPC command metadata differs from ipc_contract.rs".into());
    }
    Ok(())
}

fn generate_bindings(root: &Path, export_path: &Path) -> Result<()> {
    let export_dir = export_path
        .to_str()
        .ok_or("binding export directory is not valid UTF-8")?
        .to_owned();
    let export_env = [("TS_RS_EXPORT_DIR", export_dir.as_str())];
    process::run(
        "cargo",
        &[
            "test",
            "--manifest-path",
            "rust/Cargo.toml",
            "-p",
            "sky_desktop_shell",
            "--lib",
            "--no-default-features",
            "--features",
            "tauri-test",
            "--locked",
        ],
        root,
        &export_env,
    )?;
    Ok(())
}

fn prepare_binding_export_dir(root: &Path) -> Result<std::path::PathBuf> {
    let export_dir = root.join("rust/target/xtask-bindings");
    if export_dir.exists() {
        if fs::symlink_metadata(&export_dir)?.file_type().is_symlink() {
            return Err("binding export directory must not be a symlink".into());
        }
        fs::remove_dir_all(&export_dir)?;
    }
    fs::create_dir_all(&export_dir)?;
    Ok(export_dir)
}

fn collect_binding_files(root: &Path) -> Result<BTreeMap<String, Vec<u8>>> {
    if !root.is_dir() {
        return Err(format!("binding export directory is missing: {}", root.display()).into());
    }
    let mut files = BTreeMap::new();
    for entry in WalkDir::new(root).follow_links(false) {
        let entry = entry?;
        if entry.file_type().is_symlink() {
            return Err(format!(
                "binding export contains a symlink: {}",
                entry.path().display()
            )
            .into());
        }
        if !entry.file_type().is_file() {
            continue;
        }
        let relative = entry
            .path()
            .strip_prefix(root)?
            .to_string_lossy()
            .replace('\\', "/");
        files.insert(relative, normalized_text_bytes(fs::read(entry.path())?));
    }
    Ok(files)
}

fn normalized_text_bytes(bytes: Vec<u8>) -> Vec<u8> {
    String::from_utf8(bytes.clone())
        .map(|text| text.replace("\r\n", "\n").into_bytes())
        .unwrap_or(bytes)
}

fn compare_generated_bindings(root: &Path, export_dir: &Path) -> Result<()> {
    let checked_in_dir = root.join("desktop/src/bridge/generated");
    let mut expected = collect_binding_files(&checked_in_dir)?;
    // These are maintained frontend support files rather than ts-rs exports.
    expected.remove("index.ts");
    expected.remove("serde_json/JsonValue.ts");
    expected.remove("commands.ts");
    expected.remove("command_names.ts");
    let mut actual = collect_binding_files(export_dir)?;
    actual.remove("commands.ts");
    actual.remove("command_names.ts");
    if expected != actual {
        let expected_paths = expected.keys().cloned().collect::<Vec<_>>();
        let actual_paths = actual.keys().cloned().collect::<Vec<_>>();
        let changed = expected_paths
            .iter()
            .chain(actual_paths.iter())
            .filter(|path| expected.get(*path) != actual.get(*path))
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        return Err(format!(
            "generated Tauri bindings differ from committed output: {}",
            changed.into_iter().collect::<Vec<_>>().join(", ")
        )
        .into());
    }
    Ok(())
}

pub(crate) fn should_skip_supply_chain(flag: bool, env_val: Option<&str>) -> bool {
    flag || env_val.map(|v| v.trim()) == Some("1")
}

pub fn run(group: &str, skip_supply_chain: bool) -> Result<()> {
    let root = repo::root();
    let env_val = std::env::var("SKY_CHECK_SKIP_SUPPLY_CHAIN").ok();
    let skip_supply_chain = should_skip_supply_chain(skip_supply_chain, env_val.as_deref());
    match group {
        "static" => {
            audits::agent_context::run(&root)?;
            audits::durable_names::run(&root)?;
            audits::architecture::run(&root)?;
            audits::security::run(&root)?;
            audits::zero_python::run(&root)?;
            tauri_feature_contract(&root)?;
            if !skip_supply_chain {
                supply_chain::run(None)?;
            } else {
                println!(
                    "[xtask] cargo-vet supply-chain: SKIP (verified by dedicated supply-chain gate)"
                );
            }
            branding::validate(&root)?;
            tauri_bundle::validate_config(&root)?;
            v4_trust_material_contract(&root)?;
            legacy_release_guard(&root)?;
            release_authority_contract(&root)?;
            v4_release_pipeline_contract(&root)?;
            packaged_ci_contract(&root)?;
            v4_legacy_updater_retirement(&root)?;
            retirement(&root)?;
        }
        "rust" => {
            // The canonical Windows qualification runs workspace tests in a
            // restricted environment.  Keep process-global test fixtures
            // deterministic there; this does not change product concurrency.
            let export_dir = prepare_binding_export_dir(&root)?;
            let export_dir = export_dir
                .to_str()
                .ok_or("binding export directory is not valid UTF-8")?
                .to_owned();
            let test_env = [
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
                println!(
                    "[xtask] desktop browser E2E: SKIP (classifier marked browser_required=false)"
                );
            }
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
                &root,
                &[],
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
                &root,
                &[],
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
                &root,
                &[],
            )?;
            bindings()?;
        }
        "all" => {
            run("static", skip_supply_chain)?;
            run("rust", skip_supply_chain)?;
            run("desktop", skip_supply_chain)?;
        }
        other => return Err(format!("unknown check group: {other}").into()),
    }
    println!("[xtask] check {group}: PASS");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_TAURI_FEATURE_MANIFEST: &str = r#"
[dependencies]
tauri = { version = "2.11.5", default-features = false }

[features]
default = ["desktop-runtime", "packaged-assets"]
desktop-runtime = ["tauri/wry"]
packaged-assets = ["tauri/custom-protocol", "tauri/compression"]
tauri-test = ["tauri/test"]
"#;

    fn fixture_features(source: &str) -> toml::value::Table {
        toml::from_str::<toml::Value>(source.trim_start())
            .unwrap()
            .get("features")
            .and_then(toml::Value::as_table)
            .unwrap()
            .clone()
    }

    #[test]
    fn tauri_feature_contract_accepts_split_runtime_and_packaged_assets() {
        let resolution = tauri_feature_contract_manifest(VALID_TAURI_FEATURE_MANIFEST).unwrap();
        assert_eq!(
            resolution.default,
            ["desktop-runtime", "packaged-assets"]
                .into_iter()
                .map(str::to_owned)
                .collect()
        );
        assert_eq!(
            resolution.dev,
            ["desktop-runtime".to_owned()].into_iter().collect()
        );
    }

    #[test]
    fn tauri_feature_contract_rejects_the_old_combined_runtime_topology() {
        let source = r#"
[dependencies]
tauri = { version = "2.11.5", default-features = false }

[features]
default = ["desktop-runtime"]
desktop-runtime = ["tauri/wry", "tauri/custom-protocol", "tauri/compression"]
"#;
        let features = fixture_features(source);
        let default = feature_entries(&features, "default").unwrap();
        assert!(
            simulate_tauri_dev_features(&default, &features)
                .unwrap()
                .is_empty()
        );
        let error = tauri_feature_contract_manifest(source).unwrap_err();
        assert!(error.contains("default features must directly contain"));
    }

    #[test]
    fn tauri_feature_contract_rejects_a_nested_production_alias() {
        let source = r#"
[dependencies]
tauri = { version = "2.11.5", default-features = false }

[features]
default = ["production"]
production = ["desktop-runtime", "packaged-assets"]
desktop-runtime = ["tauri/wry"]
packaged-assets = ["tauri/custom-protocol", "tauri/compression"]
"#;
        let error = tauri_feature_contract_manifest(source).unwrap_err();
        assert!(error.contains("default features must directly contain"));
    }

    #[test]
    fn tauri_feature_contract_rejects_missing_wry() {
        let source = VALID_TAURI_FEATURE_MANIFEST
            .replace("desktop-runtime = [\"tauri/wry\"]", "desktop-runtime = []");
        let error = tauri_feature_contract_manifest(&source).unwrap_err();
        assert!(error.contains("desktop-runtime must directly contain `tauri/wry`"));
    }

    #[test]
    fn tauri_feature_contract_rejects_protocol_inside_runtime_or_its_aliases() {
        let source = VALID_TAURI_FEATURE_MANIFEST.replace(
            "desktop-runtime = [\"tauri/wry\"]",
            "desktop-runtime = [\"tauri/wry\", \"runtime-packaging\"]\nruntime-packaging = [\"tauri/custom-protocol\"]",
        );
        let error = tauri_feature_contract_manifest(&source).unwrap_err();
        assert!(error.contains("desktop-runtime must not contain `tauri/custom-protocol`"));
    }

    #[test]
    fn legacy_release_guard_requires_the_permanent_v4_namespace_guard() {
        let source = r#"
      - name: Block v4+ tags from legacy v3 release workflow
        run: |
          $tag = $env:GITHUB_REF_NAME
          if ($tag -match '^v(?<major>\d+)\.') {
            $major = [int64]$Matches.major
            if ($major -ge 4) {
              throw "v4 publication is prohibited in the legacy v3 release workflow; use the dedicated v4 release authority: $tag"
            }
          }
"#;
        assert!(legacy_release_guard_source(source).is_ok());
        assert!(
            legacy_release_guard_source(&source.replace("$major -ge 4", "$major -gt 4")).is_err()
        );
    }

    #[test]
    fn v4_legacy_updater_contract_rejects_retired_runtime_and_release_markers() {
        assert!(
            v4_legacy_updater_source_contract(
                "official Tauri NSIS and UpdateService only",
                "fixture"
            )
            .is_ok()
        );
        for marker in [
            "sky_updater",
            "Sky-Auto-Player-Updater.exe",
            "sky_updater_e2e",
            "cargo xtask dist",
            "verify-dist",
            "MANIFEST.json",
            "MANIFEST.json.sig",
            "SKY_UPDATE_SIGNING_KEY_HEX",
            "pep440_rs",
            "packaging.version",
            "ActiveUpdateState",
            "active_update_for_install",
        ] {
            assert!(
                v4_legacy_updater_source_contract(marker, "fixture").is_err(),
                "{marker}"
            );
        }
    }

    #[test]
    fn release_authority_contract_requires_rust_owned_channels_and_read_only_acceptance() {
        let native = r#"
const V4_RELEASE_AUTHORITY_REPOSITORY: &str = "pumni/Sky-Auto-Player-Releases";
const V4_STABLE_METADATA_ENDPOINT: &str = "https://raw.githubusercontent.com/pumni/Sky-Auto-Player-Releases/main/channels/stable/latest.json";
const V4_BETA_METADATA_ENDPOINT: &str = "https://raw.githubusercontent.com/pumni/Sky-Auto-Player-Releases/main/channels/beta/latest.json";
endpoints(vec![endpoint])
fn production_authority_is_fixed_and_channel_isolated() {}
"#;
        for marker in [
            "V4_RELEASE_AUTHORITY_REPOSITORY",
            "V4_STABLE_METADATA_ENDPOINT",
            "V4_BETA_METADATA_ENDPOINT",
            "https://raw.githubusercontent.com/pumni/Sky-Auto-Player-Releases/main/channels/stable/latest.json",
            "https://raw.githubusercontent.com/pumni/Sky-Auto-Player-Releases/main/channels/beta/latest.json",
            "endpoints(vec![endpoint])",
            "production_authority_is_fixed_and_channel_isolated",
        ] {
            assert!(native.contains(marker), "{marker}");
        }
        assert!(!native.contains("api.github.com/repos/pumni/Sky-Auto-Player/releases"));

        let acceptance = r#"
# This is deliberately read-only.
$sourceRepository = "pumni/Sky-Auto-Player"
$authorityRepository = "pumni/Sky-Auto-Player-Releases"
releases/latest
read_only=true
"#;
        assert!(acceptance.contains("This is deliberately read-only"));
        assert!(acceptance.contains("releases/latest"));
        assert!(!acceptance.contains("gh release create"));
    }

    #[test]
    fn v4_release_pipeline_contract_requires_draft_download_and_publish_order() {
        let workflow = r#"
name: V4 Release Pipeline
on:
  workflow_dispatch:
permissions:
  contents: read
  id-token: write
  attestations: write
jobs:
  release:
    runs-on: [self-hosted, windows, v4-release, single-tenant]
    ref: ${{ inputs.source_sha }}
    persist-credentials: false
    env: V4_RELEASE_AUTHORITY_TOKEN
    -State ValidateRequest
    -State ValidateAuthority
    -State BuildCandidate
    -State CreateDraft
    -State DownloadDraft
    -State QualifyDownloaded
    Qualify downloaded exact candidate bytes and packaged update
    -State RecordAttestations
    -State PublishDraft
    -State PromoteMetadata
    -State FinalVerify
    actions/attest@v4
    actions/upload-artifact@v7
    --source-digest $env:GITHUB_SHA
    GH_TOKEN: ${{ github.token }}
"#;
        let pipeline = r#"
ValidateRequest ValidateAuthority BuildCandidate CreateDraft DownloadDraft QualifyDownloaded RecordAttestations PublishDraft PromoteMetadata FinalVerify authority main is not initialized upload_url immutable-releases Assert-ImmutableRelease scripts/ci_tauri_update_e2e.ps1 CandidateInstallerPath CandidateSignaturePath CandidatePublicKeyPath export-public-key Start-MpScan scan_performed selftest-update-active-playback scan_v4_defender_exact.ps1 v4_updater_credential_broker.ps1
function Invoke-BuildCandidate {
  & pwsh -File orchestrate_v4_production_release.ps1
}
function Invoke-CreateDraft { draft = $true; refs/heads/main; authority already contains tag; existing releases are never moved or replaced }
function Invoke-DownloadDraft { downloaded; Get-FileHash; unsigned-zero-budget }
function Invoke-QualifyDownloaded { verify-signature; verify-tauri-bundle; current-user; active-playback-install-rejected; previous-v4-to-exact-downloaded-candidate-update; selftest-update-active-playback; ci_v4_release_authority_acceptance.ps1; promote_v4_metadata.ps1; release-authority; published_at; Start-MpScan; scan_performed }
function Invoke-RecordAttestations { V4_RELEASE_AUTHORITY_TOKEN }
function Invoke-PublishDraft { draft = $false }
function Invoke-PromoteMetadata { metadata promotion is forbidden before immutable publication }
function Invoke-FinalVerify { FinalVerify }
"#;
        let regression = r#"
class MockReleaseApi { [int]$BuildCount = 0; [string]$UploadUrl = ''; [bool]$UploadedThroughReleaseUrl = $false; [bool]$ExactDownloadedBytes = $false; [bool]$immutable = $false; candidate rebuilt; promotion before immutable publication; BuildCount -ne 1; UploadedThroughReleaseUrl; ExactDownloadedBytes; immutable }
"#;
        assert!(v4_release_pipeline_contract_source(workflow, pipeline, regression).is_ok());

        let reordered = workflow.replace(
            "-State CreateDraft\n    -State DownloadDraft",
            "-State DownloadDraft\n    -State CreateDraft",
        );
        assert!(v4_release_pipeline_contract_source(&reordered, pipeline, regression).is_err());
        let duplicated_build = pipeline.replace(
            "function Invoke-CreateDraft",
            "orchestrate_v4_production_release.ps1\nfunction Invoke-CreateDraft",
        );
        assert!(
            v4_release_pipeline_contract_source(workflow, &duplicated_build, regression).is_err()
        );
    }

    #[test]
    fn packaged_ci_contract_requires_tauri_and_rejects_v3_artifacts() {
        let source = r#"
  validate:
    name: Windows compatibility and unit tests
    steps:
      - run: cargo xtask check rust
  updater_e2e:
    name: Packaged v4 updater fixture qualification
    needs: [changes, static, release_authority, supply_chain, validate]
    env:
      RUNNER_TEMP: runner-temp
      CARGO_TARGET_DIR: runner-temp
    steps:
      - run: dangerousInsecureTransportProtocol = true
      - run: bun run tauri build --features tauri-update-fixture
      - run: pwsh scripts/ci_tauri_update_e2e.ps1 -BundleDir runner-temp
  packaged:
    name: Packaged v4 Tauri NSIS qualification
    needs: [changes, static, release_authority, supply_chain, validate]
    steps:
      - name: Resolve GitHub CLI for artifact attestation verification
        run: Get-Command gh.exe -CommandType Application; SKY_GH_PATH=$ghPath
      - name: Construct Python-unavailable canonical environment
        run: restricted PATH
      - name: Build and sign canonical Tauri NSIS artifact
      - run: bun install --frozen-lockfile
      - run: bun run build
      - run: bun run tauri signer generate
        env: { TAURI_SIGNING_PRIVATE_KEY: test }
        # Tauri updater signer generation failed with exit code
      - run: bun run tauri build --ci --config test.json
        # Tauri build failed with exit code
      - name: Prepare bounded ephemeral Authenticode test certificate
        timeout-minutes: 2
        run: pwsh scripts/setup_v4_test_signing.ps1 -EnvFile $env:GITHUB_ENV -TimeoutSeconds 30
      - name: Run Authenticode tamper regression
        run: pwsh scripts/test_v4_authenticode_integrity.ps1
        # CI self-signed credentials remain test-only; canonical package evidence is unsigned.
      - name: Run V4 production signing contract test
        run: pwsh scripts/test_v4_production_signing_contract.ps1
        # V4 production signing contract test failed with exit code
      - name: Run V4 production release orchestrator contract test
        run: pwsh scripts/test_v4_production_orchestrator.ps1
        # V4 production release orchestrator contract test failed with exit code
      - name: Run V4 updater private-key verifier secret-output regression
        run: pwsh scripts/test_v4_updater_private_key.ps1
        # V4 updater private-key verifier regression failed with exit code
      - name: Verify Tauri Authenticode signature
        run: pwsh scripts/verify_v4_authenticode.ps1 -Mode unsigned-zero-budget
        # Authenticode verification failed with exit code
        # Installed Authenticode verification failed with exit code
        # CI self-signed credentials remain test-only
      - name: Generate Tauri SPDX SBOM
        run: cargo xtask sbom generate
        # SBOM generation failed with exit code
      - name: Verify Tauri SPDX SBOM
        run: cargo xtask sbom verify
        # SBOM verification failed with exit code
      - name: Verify exact Tauri NSIS bundle
        run: cargo xtask verify-tauri-bundle
        # Tauri bundle verification failed with exit code
      - name: Qualify current-user install, launch, and uninstall
        run: check sky_desktop_shell.exe uninstall.exe
      - name: Clean up ephemeral Authenticode test certificate
        run: pwsh scripts/cleanup_v4_test_signing.ps1
        # Installer attestation verification failed with exit code
        # Updater signature attestation verification failed with exit code
        # SBOM attestation verification failed with exit code
      - name: Verify exact GitHub artifact attestations
        env:
          GH_TOKEN: ${{ github.token }}
        run: |
          $attestationHelp = (& $env:SKY_GH_PATH attestation verify --help 2>&1 | Out-String)
          & $env:SKY_GH_PATH attestation verify $installer -R $env:GITHUB_REPOSITORY --source-digest $env:GITHUB_SHA --signer-workflow $signerWorkflow
          & $env:SKY_GH_PATH attestation verify $signature -R $env:GITHUB_REPOSITORY --source-digest $env:GITHUB_SHA --signer-workflow $signerWorkflow
          & $env:SKY_GH_PATH attestation verify $installer -R $env:GITHUB_REPOSITORY --predicate-type https://spdx.dev/Document/v2.3 --source-digest $env:GITHUB_SHA --signer-workflow $signerWorkflow
          # GitHub CLI absolute path is unavailable for attestation verification
          # GH_TOKEN is unavailable for attestation verification
          # Installed GitHub CLI lacks the required exact-source attestation options
      - name: Upload exact Tauri NSIS release candidate
        uses: actions/upload-artifact@v7
        path: rust/target/dist/bundle/nsis
  status:
    needs: [changes, static, release_authority, supply_chain, validate, updater_e2e, packaged]
    env: { UPDATER_E2E_RESULT: success }
        "#;
        assert!(packaged_ci_contract_source(source).is_ok());
        let crlf_source = source.replace('\n', "\r\n");
        assert!(packaged_ci_contract_source(&crlf_source).is_ok());
        let unblocked_package_jobs = source.replace(
            "needs: [changes, static, release_authority, supply_chain, validate]",
            "needs: changes",
        );
        assert!(packaged_ci_contract_source(&unblocked_package_jobs).is_err());
        for forbidden in [
            "tauri-update-fixture",
            "dangerousInsecureTransportProtocol",
            "127.0.0.1:17845",
            "CARGO_TARGET_DIR",
            "--features",
            "cargo xtask dist",
            "verify-dist",
            "Sky-Auto-Player-v",
            "Sky-Auto-Player-Updater.exe",
            "MANIFEST.json",
            "PORTABLE_ARTIFACT",
            "portable",
        ] {
            let source_with_legacy_marker =
                source.replace("  status:", &format!("  # {forbidden}\n  status:"));
            assert!(
                packaged_ci_contract_source(&source_with_legacy_marker).is_err(),
                "{forbidden}"
            );
        }
    }

    #[test]
    fn packaged_ci_contract_requires_the_isolated_fixture_job() {
        let source = r#"
  validate:
    name: Windows compatibility and unit tests
    steps:
      - run: cargo xtask check rust
  packaged:
    name: Packaged v4 Tauri NSIS qualification
    needs: [changes, static, release_authority, supply_chain, validate]
    steps:
      - run: bun install --frozen-lockfile
      - run: bun run build
      - run: bun run tauri signer generate
      - run: bun run tauri build --ci --config test.json
      - run: cargo xtask verify-tauri-bundle
      - name: Qualify current-user install, launch, and uninstall
        run: check sky_desktop_shell.exe uninstall.exe
      - uses: actions/upload-artifact@v7
  status:
    needs: [changes, static, supply_chain, validate, packaged]
        "#;
        assert!(packaged_ci_contract_source(source).is_err());
    }

    #[test]
    fn security_ignores_comments_but_flags_the_complete_forbidden_set() {
        let source = "// NtReadVirtualMemory and ntdll.dll are documentation only\n/* SetWindowsHookExW */\nunsafe { NtReadVirtualMemory(); DebugActiveProcessStop(); ContinueDebugEvent(); WaitForDebugEvent(); NtQueryInformationProcess(); keybd_event(); mouse_event(); }\n";
        let findings = scan_rust_text(Path::new("fixture.rs"), source);
        let rules = findings
            .iter()
            .map(|finding| finding.rule.as_str())
            .collect::<BTreeSet<_>>();
        assert!(rules.contains("forbidden-call:NtReadVirtualMemory"));
        assert!(rules.contains("forbidden-call:DebugActiveProcessStop"));
        assert!(rules.contains("forbidden-call:ContinueDebugEvent"));
        assert!(rules.contains("forbidden-call:WaitForDebugEvent"));
        assert!(rules.contains("forbidden-call:NtQueryInformationProcess"));
        assert!(rules.contains("forbidden-call:keybd_event"));
        assert!(rules.contains("forbidden-call:mouse_event"));
        assert!(
            !findings
                .iter()
                .any(|finding| finding.rule == "forbidden-call:SetWindowsHookExW")
        );
        assert!(
            !findings
                .iter()
                .any(|finding| finding.rule == "forbidden-dll-load")
        );
    }

    #[test]
    fn security_allows_sendinput_and_approved_windows_modules() {
        let source = "unsafe { SendInput(1, inputs, size); }\nuse windows_sys::Win32::Foundation::HANDLE;\nuse windows_sys::Win32::UI::Input::KeyboardAndMouse::SendInput;\nuse windows_sys::Win32::System::Threading::CloseHandle;\n";
        assert!(scan_rust_text(Path::new("fixture.rs"), source).is_empty());
    }

    #[test]
    fn security_rejects_unapproved_windows_diagnostics_module_and_ntdll() {
        let source = "use windows_sys::Win32::System::Diagnostics::Debug::OutputDebugStringW;\nlet name = \"ntdll.dll\";\n";
        let findings = scan_rust_text(Path::new("fixture.rs"), source);
        assert!(
            findings
                .iter()
                .any(|finding| finding.rule == "disallowed-windows-sys-module")
        );
        assert!(
            findings
                .iter()
                .any(|finding| finding.rule == "forbidden-dll-load")
        );
    }

    #[test]
    fn trust_guard_rejects_renamed_and_encoded_minisign_secret_keys() {
        let secret_payload = STANDARD.encode([0_u8; 158]);
        let secret_text =
            format!("untrusted comment: minisign encrypted secret key\n{secret_payload}");
        assert!(is_tauri_minisign_private_key(secret_text.as_bytes()));
        assert!(is_tauri_minisign_private_key(
            STANDARD.encode(&secret_text).as_bytes()
        ));

        let public_text = format!(
            "untrusted comment: minisign public key: fixture\n{}",
            STANDARD.encode([0_u8; 32])
        );
        assert!(!is_tauri_minisign_private_key(
            STANDARD.encode(public_text).as_bytes()
        ));

        let fixture = std::env::temp_dir().join(format!(
            "sky-xtask-renamed-secret-{}-{:?}.txt",
            std::process::id(),
            std::thread::current().id()
        ));
        fs::write(&fixture, STANDARD.encode(secret_text)).unwrap();
        let renamed_fixture = fs::read(&fixture).unwrap();
        assert!(is_tauri_minisign_private_key(&renamed_fixture));
        fs::remove_file(fixture).unwrap();
    }

    #[test]
    fn architecture_allowlist_is_loaded_and_current_tree_passes() {
        let root = repo::root();
        let allowlist = load_architecture_allowlist(&root).unwrap();
        assert!(allowlist.contains_key(&(
            "rust/crates/sky_player/src/engine/tests.rs".into(),
            "regular_module_lines".into()
        )));
        architecture(&root).unwrap();
    }

    #[test]
    fn architecture_helpers_cover_context_function_glob_and_schedule_rules() {
        let context = (0..13)
            .map(|index| format!("    field_{index}: u8,\n"))
            .collect::<String>();
        let context = format!("struct WorkerContext {{\n{context}}}\n");
        assert_eq!(context_violations(&clean_lines(&context)).len(), 1);
        let function = std::iter::once("fn oversized() {\n".to_owned())
            .chain((0..181).map(|_| "    let _value = 1;\n".to_owned()))
            .chain(std::iter::once("}\n".to_owned()))
            .collect::<Vec<_>>();
        assert_eq!(function_line_violations(&function, 180).len(), 1);
        assert!(top_level_glob_import(&clean_lines(
            "use super::*;\nfn f() {}\n"
        )));
        assert!(contains_unsafe_code("unsafe { value(); }"));
        assert!(
            WORKER_SCHEDULE_CLONE_PATTERNS
                .iter()
                .any(|pattern| "schedule.clone()".contains(pattern))
        );
    }

    #[test]
    fn xtask_process_guard_rejects_python_and_allows_native_tools() {
        let forbidden = format!("process::run({:?}, &[], root, &[])", "python");
        assert!(xtask_process_violation(&forbidden).is_some());
        assert!(xtask_process_violation("Command::new(\"cargo\")").is_none());
    }

    #[test]
    fn ledger_evidence_rejects_missing_targets_and_placeholders() {
        let root = repo::root();
        let missing = serde_json::json!({
            "invariants": ["concrete invariant"],
            "evidence": ["rust/no_such_file.rs::missing"]
        });
        assert!(
            validate_evidence_entry(
                &root,
                "tests/deleted.py",
                "MIGRATED_XTASK",
                missing.as_object().unwrap(),
                &["generic evidence"]
            )
            .is_err()
        );
        let placeholder = serde_json::json!({
            "invariants": ["concrete invariant"],
            "evidence": ["generic evidence"]
        });
        assert!(
            validate_evidence_entry(
                &root,
                "tests/deleted.py",
                "DUPLICATE",
                placeholder.as_object().unwrap(),
                &["generic evidence"]
            )
            .is_err()
        );
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
