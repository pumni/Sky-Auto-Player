[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$classifierPath = Join-Path $PSScriptRoot "ci_classify.ps1"
$tempRoot = Join-Path ([IO.Path]::GetTempPath()) ("sky-ci-classify-" + [guid]::NewGuid().ToString("N"))

function Fail([string]$Message) {
    throw "CI classifier self-test failed: $Message"
}

function Invoke-Classification([string[]]$Paths, [switch]$Full, [string]$Name) {
    $pathsFile = Join-Path $tempRoot "$Name.paths"
    $Paths | Set-Content -LiteralPath $pathsFile -Encoding utf8
    $arguments = @(
        "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
        "-File", $classifierPath, "-PathsFile", $pathsFile
    )
    if ($Full) {
        $arguments = @(
            "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
            "-File", $classifierPath, "-Full"
        )
    }
    $lines = @(& pwsh @arguments)
    if ($LASTEXITCODE -ne 0) { Fail "$Name exited with $LASTEXITCODE" }
    $result = [ordered]@{}
    foreach ($line in $lines) {
        $parts = ([string]$line).Split('=', 2)
        if ($parts.Count -ne 2) { Fail "$Name emitted malformed output: $line" }
        [void]($result[$parts[0]] = $parts[1])
    }
    $expectedNames = @(
        "rust_required", "desktop_required", "desktop_e2e_required", "package_required",
        "updater_required", "release_required", "supply_chain_required", "site_required",
        "classification_reason"
    )
    if (($result.Keys -join "|") -cne ($expectedNames -join "|")) {
        Fail "$Name emitted unexpected output names: $($result.Keys -join ', ')"
    }
    return ,$result
}

function Assert-Result([string]$Name, $Expected, [string[]]$Paths) {
    $actual = Invoke-Classification $Paths -Name $Name
    foreach ($key in $Expected.Keys) {
        $expectedValue = "$($Expected[$key])"
        if ("$($actual.Item($key))" -ne $expectedValue) {
            Fail "$Name expected $key=$expectedValue but got $($actual.Item($key))"
        }
    }
}

function Assert-ReasonContains([string]$Name, [string[]]$Paths, [string]$ExpectedText) {
    $actual = Invoke-Classification $Paths -Name $Name
    if (-not ([string]$actual.classification_reason).Contains($ExpectedText)) {
        Fail "$Name expected classification_reason to contain '$ExpectedText' but got '$($actual.classification_reason)'"
    }
}

try {
    New-Item -ItemType Directory -Path $tempRoot -Force | Out-Null

    $falseLanes = @{
        rust_required = "false"; desktop_required = "false"; desktop_e2e_required = "false";
        package_required = "false"; updater_required = "false"; release_required = "false";
        supply_chain_required = "false"; site_required = "false"
    }
    Assert-Result "readme" $falseLanes @("README.md")
    Assert-Result "docs-evidence" $falseLanes @("docs/evidence/foo.png")
    Assert-ReasonContains "static-only-with-readme" @(".config/rust_architecture_allowlist.json", "README.md") "static-only"
    Assert-Result "static-only-with-site" (@{ site_required = "true" }) @(".config/rust_architecture_allowlist.json", "site/src/pages/index.astro")
    Assert-ReasonContains "static-only-with-site-reason" @(".config/rust_architecture_allowlist.json", "site/src/pages/index.astro") "static-only"
    Assert-Result "release-notes" (@{ release_required = "true" }) @("docs/releases/v4.0.2.md")
    Assert-Result "site" (@{ site_required = "true" }) @("site/src/pages/index.astro")
    Assert-Result "desktop-frontend" (@{ desktop_required = "true"; desktop_e2e_required = "true"; package_required = "false" }) @("desktop/src/App.tsx")
    Assert-Result "desktop-bridge" (@{ desktop_required = "true"; desktop_e2e_required = "true"; package_required = "false" }) @("desktop/src/bridge/tauriBridge.ts")
    Assert-Result "desktop-commands" (@{ rust_required = "true"; desktop_required = "true"; package_required = "false" }) @("desktop/src-tauri/src/commands.rs")
    Assert-Result "native-runtime" (@{ rust_required = "true"; desktop_required = "true"; updater_required = "false"; package_required = "false" }) @("desktop/src-tauri/src/native_runtime.rs")
    Assert-Result "native-update" (@{ rust_required = "true"; desktop_required = "true"; updater_required = "true"; package_required = "false" }) @("desktop/src-tauri/src/native_update.rs")
    Assert-Result "main" (@{ rust_required = "true"; desktop_required = "true"; package_required = "true"; updater_required = "false" }) @("desktop/src-tauri/src/main.rs")
    Assert-Result "lib" (@{ rust_required = "true"; desktop_required = "true"; package_required = "true"; updater_required = "false" }) @("desktop/src-tauri/src/lib.rs")
    Assert-Result "tauri-config" (@{ desktop_required = "true"; package_required = "true"; updater_required = "true"; rust_required = "false" }) @("desktop/src-tauri/tauri.conf.json")
    Assert-Result "icon" (@{ package_required = "true"; desktop_required = "false" }) @("desktop/src-tauri/icons/icon.ico")
    Assert-Result "songs" (@{ package_required = "true"; desktop_required = "false" }) @("songs/foo.json")
    Assert-Result "builtin-songs" (@{ package_required = "true"; desktop_required = "false" }) @("builtin-songs/manifest.json")
    Assert-Result "rust-source" (@{ rust_required = "true"; desktop_required = "false"; package_required = "false" }) @("rust/crates/sky_player/src/lib.rs")
    Assert-Result "rust-manifest" (@{ rust_required = "true"; supply_chain_required = "true"; package_required = "false" }) @("rust/crates/sky_player/Cargo.toml")
    Assert-Result "desktop-package" (@{ desktop_required = "true"; desktop_e2e_required = "true"; package_required = "true"; updater_required = "true"; supply_chain_required = "true" }) @("desktop/package.json")
    Assert-Result "cargo-lock" (@{ rust_required = "true"; package_required = "true"; updater_required = "true"; release_required = "true"; supply_chain_required = "true" }) @("rust/Cargo.lock")
    Assert-Result "updater-script" (@{ updater_required = "true"; rust_required = "false"; package_required = "false" }) @("scripts/ci_tauri_update_e2e_core.ps1")
    Assert-Result "release-script" (@{ release_required = "true"; updater_required = "false"; rust_required = "false" }) @("scripts/v4_release_pipeline.ps1")
    Assert-Result "metadata-promotion" (@{ updater_required = "true"; release_required = "true" }) @("scripts/promote_v4_metadata.ps1")
    Assert-Result "pages-workflow" (@{ site_required = "true"; release_required = "false" }) @(".github/workflows/pages.yml")
    Assert-Result "release-workflow" (@{ release_required = "true"; site_required = "false" }) @(".github/workflows/release-v4.yml")
    Assert-Result "rehearsal-workflow" (@{ release_required = "true"; site_required = "false" }) @(".github/workflows/rehearse-v4.yml")
    Assert-Result "ci-workflow" (@{ rust_required = "true"; desktop_required = "true"; desktop_e2e_required = "true"; package_required = "true"; updater_required = "true"; release_required = "true"; supply_chain_required = "true"; site_required = "true" }) @(".github/workflows/ci.yml")
    Assert-Result "classifier" (@{ rust_required = "true"; desktop_required = "true"; desktop_e2e_required = "true"; package_required = "true"; updater_required = "true"; release_required = "true"; supply_chain_required = "true"; site_required = "true" }) @("scripts/ci_classify.ps1")
    Assert-Result "unknown-workflow" (@{ rust_required = "true"; desktop_required = "true"; desktop_e2e_required = "true"; package_required = "true"; updater_required = "true"; release_required = "true"; supply_chain_required = "true"; site_required = "true" }) @(".github/workflows/unknown.yml")
    $full = Invoke-Classification @() -Full -Name "manual-full"
    foreach ($name in @("rust_required", "desktop_required", "desktop_e2e_required", "package_required", "updater_required", "release_required", "supply_chain_required", "site_required")) {
        if ($full.Item($name) -ne "true") { Fail "manual-full did not request full validation" }
    }

    $invalidArguments = @(
        "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
        "-File", $classifierPath, "-BaseSha", ("0" * 40), "-HeadSha", ("1" * 40)
    )
    $invalidOutput = @(& pwsh @invalidArguments)
    $invalidReasons = @($invalidOutput | Where-Object { $_ -like "classification_reason=classifier failure*" })
    if ($LASTEXITCODE -ne 0 -or $invalidOutput -notcontains "rust_required=true" -or
        $invalidReasons.Count -ne 1) {
        Fail "invalid or unusable base SHA did not fail closed to full validation"
    }

    Write-Output "CI classifier self-tests: PASS"
} finally {
    if (Test-Path -LiteralPath $tempRoot) {
        Remove-Item -LiteralPath $tempRoot -Recurse -Force -ErrorAction SilentlyContinue
    }
}
