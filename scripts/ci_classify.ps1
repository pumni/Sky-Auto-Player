[CmdletBinding()]
param(
    [Alias("Base")]
    [string]$BaseSha,

    [Alias("Head")]
    [string]$HeadSha,

    [string]$PathsFile,

    [switch]$Full
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$outputNames = @(
    "rust_required",
    "desktop_required",
    "desktop_e2e_required",
    "package_required",
    "updater_required",
    "release_required",
    "supply_chain_required",
    "site_required"
)

function New-Classification([string]$Reason) {
    $classification = [ordered]@{}
    foreach ($name in $outputNames) {
        [void]($classification[$name] = $false)
    }
    [void]($classification.classification_reason = $Reason)
    return $classification
}

function New-FullClassification([string]$Reason) {
    $classification = New-Classification $Reason
    foreach ($name in $outputNames) {
        [void]($classification[$name] = $true)
    }
    return $classification
}

function Write-Classification([System.Collections.IDictionary]$Classification) {
    foreach ($name in $outputNames) {
        $value = [bool]$Classification[$name]
        Write-Output "$name=$($value.ToString().ToLowerInvariant())"
    }
    Write-Output "classification_reason=$([string]$Classification.classification_reason)"
}

function Normalize-Path([string]$Path) {
    $normalized = $Path.Trim().Replace('\', '/')
    while ($normalized.StartsWith('./', [StringComparison]::Ordinal)) {
        $normalized = $normalized.Substring(2)
    }
    return $normalized
}

function Add-Lane([System.Collections.IDictionary]$Classification, [string]$Lane) {
    [void]($Classification[$Lane] = $true)
}

function Add-Lanes([System.Collections.IDictionary]$Classification, [string[]]$Lanes) {
    foreach ($lane in $Lanes) {
        Add-Lane $Classification $lane
    }
}

function Get-DisplayPaths([string[]]$Paths) {
    return ($Paths | Select-Object -First 3) -join ', '
}

function Get-ClassificationForPaths([string[]]$RawPaths) {
    $paths = @(
        $RawPaths |
            ForEach-Object { Normalize-Path ([string]$_) } |
            Where-Object { -not [string]::IsNullOrWhiteSpace($_) }
    )
    if ($paths.Count -eq 0) {
        return New-Classification "no changed paths"
    }

    $classification = New-Classification "classified changed paths: $(Get-DisplayPaths $paths)"
    $unknownPaths = [System.Collections.Generic.List[string]]::new()
    $staticOnlyPaths = [System.Collections.Generic.List[string]]::new()

    foreach ($path in $paths) {
        if ($path -eq "README.md" -or $path -eq "CHANGELOG.md" -or
            ($path.StartsWith("docs/", [StringComparison]::Ordinal) -and
                -not $path.StartsWith("docs/releases/", [StringComparison]::Ordinal))) {
            continue
        }

        if ($path.StartsWith("docs/releases/", [StringComparison]::Ordinal)) {
            Add-Lane $classification "release_required"
            continue
        }

        if ($path.StartsWith("site/", [StringComparison]::Ordinal) -or
            $path.StartsWith(".github/actions/site-validate/", [StringComparison]::Ordinal) -or
            $path -eq ".github/workflows/pages.yml") {
            Add-Lane $classification "site_required"
            continue
        }

        if ($path -eq ".github/workflows/ci.yml" -or
            ($path.StartsWith(".github/workflows/", [StringComparison]::Ordinal) -and
                $path -notin @(
                    ".github/workflows/pages.yml",
                    ".github/workflows/release-v4.yml",
                    ".github/workflows/rehearse-v4.yml"
                )) -or
            ($path.StartsWith(".github/actions/", [StringComparison]::Ordinal) -and
                -not $path.StartsWith(".github/actions/site-validate/", [StringComparison]::Ordinal))) {
            $unknownPaths.Add($path)
            continue
        }

        if ($path -in @(
                ".github/workflows/release-v4.yml",
                ".github/workflows/rehearse-v4.yml"
            )) {
            Add-Lane $classification "release_required"
            continue
        }

        if ($path -eq ".config/rust_architecture_allowlist.json") {
            $staticOnlyPaths.Add($path)
            continue
        }
        if ($path -eq ".config/security_audit_baseline.json") {
            Add-Lane $classification "supply_chain_required"
            continue
        }

        if ($path -eq "desktop/src-tauri/Cargo.toml") {
            Add-Lanes $classification @(
                "rust_required", "desktop_required", "package_required",
                "updater_required", "release_required", "supply_chain_required", "site_required"
            )
            continue
        }
        if ($path -eq "desktop/src-tauri/src/native_update.rs") {
            Add-Lanes $classification @("rust_required", "desktop_required", "updater_required")
            continue
        }
        if ($path -eq "desktop/src-tauri/src/main.rs" -or
            $path -eq "desktop/src-tauri/src/lib.rs") {
            Add-Lanes $classification @("rust_required", "desktop_required", "package_required")
            continue
        }
        if ($path -eq "desktop/src-tauri/build.rs") {
            Add-Lanes $classification @("rust_required", "desktop_required", "package_required", "updater_required")
            continue
        }
        if ($path -eq "desktop/src-tauri/tauri.conf.json") {
            Add-Lanes $classification @("desktop_required", "package_required", "updater_required")
            continue
        }
        if ($path.StartsWith("desktop/src-tauri/capabilities/", [StringComparison]::Ordinal)) {
            Add-Lanes $classification @("desktop_required", "package_required")
            continue
        }
        if ($path.StartsWith("desktop/src-tauri/icons/", [StringComparison]::Ordinal)) {
            Add-Lane $classification "package_required"
            continue
        }
        if ($path.StartsWith("desktop/src-tauri/src/", [StringComparison]::Ordinal)) {
            Add-Lanes $classification @("rust_required", "desktop_required")
            continue
        }
        if ($path -eq "desktop/package.json" -or $path -eq "desktop/bun.lock") {
            Add-Lanes $classification @(
                "desktop_required", "desktop_e2e_required", "package_required",
                "updater_required", "supply_chain_required"
            )
            continue
        }
        if ($path.StartsWith("desktop/", [StringComparison]::Ordinal)) {
            Add-Lanes $classification @("desktop_required", "desktop_e2e_required")
            continue
        }

        if ($path -eq "Cargo.toml") {
            Add-Lanes $classification @("rust_required", "package_required", "updater_required", "release_required", "supply_chain_required")
            continue
        }
        if ($path -eq "rust/Cargo.toml" -or $path -eq "rust/Cargo.lock") {
            Add-Lanes $classification @("rust_required", "package_required", "updater_required", "release_required", "supply_chain_required")
            continue
        }
        if ($path -eq "rust/rust-toolchain.toml") {
            Add-Lanes $classification @("rust_required", "desktop_required", "package_required", "updater_required", "release_required")
            continue
        }
        if ($path.StartsWith(".cargo/", [StringComparison]::Ordinal)) {
            Add-Lanes $classification @("rust_required", "desktop_required", "package_required", "updater_required", "release_required")
            continue
        }
        if ($path.StartsWith("rust/crates/", [StringComparison]::Ordinal)) {
            if ($path -match '^rust/crates/[^/]+/Cargo\.toml$') {
                Add-Lanes $classification @("rust_required", "supply_chain_required")
            } else {
                Add-Lane $classification "rust_required"
            }
            continue
        }
        if ($path.StartsWith("rust/xtask/", [StringComparison]::Ordinal)) {
            if ($path -eq "rust/xtask/src/tauri_bundle.rs" -or
                $path -eq "rust/xtask/src/sbom.rs" -or
                $path -eq "rust/xtask/src/builtin_catalog.rs") {
                Add-Lanes $classification @("rust_required", "package_required")
            } elseif ($path -eq "rust/xtask/src/release_metadata.rs") {
                Add-Lanes $classification @("updater_required", "release_required")
            } elseif ($path -eq "rust/xtask/src/supply_chain.rs") {
                Add-Lane $classification "supply_chain_required"
            } else {
                Add-Lane $classification "rust_required"
            }
            continue
        }
        if ($path.StartsWith("tests/", [StringComparison]::Ordinal)) {
            Add-Lane $classification "rust_required"
            continue
        }

        if ($path.StartsWith("songs/", [StringComparison]::Ordinal) -or
            $path.StartsWith("builtin-songs/", [StringComparison]::Ordinal)) {
            Add-Lane $classification "package_required"
            continue
        }

        if ($path -eq "scripts/promote_v4_metadata.ps1") {
            Add-Lanes $classification @("updater_required", "release_required")
            continue
        }
        if ($path -match '^scripts/(ci_tauri_update_e2e(?:_core)?|test_v4_updater_[^/]+|verify_v4_updater_[^/]+|updater_[^/]+)\.ps1$') {
            Add-Lane $classification "updater_required"
            continue
        }
        if ($path -match '^scripts/(sign_v4_authenticode|verify_v4_authenticode|v4_authenticode_crypto|setup_v4_test_signing|cleanup_v4_test_signing|test_v4_authenticode|test_v4_production_signing_contract)\.ps1$') {
            Add-Lane $classification "package_required"
            continue
        }
        if ($path -match '^scripts/(v4_release_[^/]+|test_v4_release_[^/]+|orchestrate_v4_production_release|verify_v4_release_runner|cleanup_v4_release_state|cleanup_v4_draft_rehearsal|v4_draft_rehearsal_external_state|v4_release_draft_lookup|test_v4_production_orchestrator|test_v4_production_topology_rehearsal)\.ps1$') {
            Add-Lane $classification "release_required"
            continue
        }
        if ($path -eq "scripts/ci_classify.ps1" -or $path -eq "scripts/test_ci_classify.ps1") {
            $unknownPaths.Add($path)
            continue
        }
        if ($path.StartsWith("scripts/", [StringComparison]::Ordinal)) {
            Add-Lanes $classification @("rust_required", "desktop_required")
            continue
        }

        $unknownPaths.Add($path)
    }

    if ($unknownPaths.Count -gt 0) {
        return New-FullClassification "unknown path requires full validation: $(Get-DisplayPaths $unknownPaths)"
    }
    $lanes = @($outputNames | Where-Object { [bool]$classification[$_] })
    if ($staticOnlyPaths.Count -gt 0 -and $lanes.Count -eq 0) {
        [void]($classification.classification_reason = "static-only plus docs/site only: $(Get-DisplayPaths $paths)")
    } elseif ($staticOnlyPaths.Count -gt 0) {
        [void]($classification.classification_reason = "static-only plus required lanes ($($lanes -join ', ')): $(Get-DisplayPaths $paths)")
    } elseif ($lanes.Count -eq 0) {
        [void]($classification.classification_reason = "docs/site only: $(Get-DisplayPaths $paths)")
    } else {
        [void]($classification.classification_reason = "required lanes ($($lanes -join ', ')): $(Get-DisplayPaths $paths)")
    }
    return $classification
}

function Test-CommitAvailable([string]$Sha) {
    $null = & git cat-file -e "$Sha^{commit}" 2>$null
    if ($LASTEXITCODE -eq 0) {
        return $true
    }

    $null = & git fetch --no-tags --depth=1 origin $Sha 2>$null
    if ($LASTEXITCODE -ne 0) {
        return $false
    }
    $null = & git cat-file -e "$Sha^{commit}" 2>$null
    return $LASTEXITCODE -eq 0
}

function Get-ChangedPathsFromDiff {
    if ([string]::IsNullOrWhiteSpace($BaseSha) -or [string]::IsNullOrWhiteSpace($HeadSha)) {
        throw "base and head SHAs are required"
    }
    foreach ($sha in @($BaseSha, $HeadSha)) {
        if ($sha -notmatch '^[0-9a-fA-F]{40}$' -or $sha -match '^0{40}$') {
            throw "base or head SHA is absent, malformed, or all-zero"
        }
        if (-not (Test-CommitAvailable $sha)) {
            throw "commit $sha is unavailable and could not be fetched"
        }
    }

    $paths = @(& git diff --name-only $BaseSha $HeadSha 2>$null)
    if ($LASTEXITCODE -ne 0) {
        throw "git diff failed for the requested base/head range"
    }
    return $paths
}

function Get-InputPaths {
    if (-not [string]::IsNullOrWhiteSpace($PathsFile)) {
        if (-not (Test-Path -LiteralPath $PathsFile -PathType Leaf)) {
            throw "paths file does not exist"
        }
        return @(Get-Content -LiteralPath $PathsFile -ErrorAction Stop)
    }
    return Get-ChangedPathsFromDiff
}

try {
    if ($Full) {
        Write-Classification (New-FullClassification "full validation requested")
        exit 0
    }
    if (-not [string]::IsNullOrWhiteSpace($PathsFile) -and
        (-not [string]::IsNullOrWhiteSpace($BaseSha) -or -not [string]::IsNullOrWhiteSpace($HeadSha))) {
        throw "paths file cannot be combined with base/head SHAs"
    }
    if ([string]::IsNullOrWhiteSpace($PathsFile) -and
        ([string]::IsNullOrWhiteSpace($BaseSha) -xor [string]::IsNullOrWhiteSpace($HeadSha))) {
        throw "base and head SHAs must be supplied together"
    }
    Write-Classification (Get-ClassificationForPaths (Get-InputPaths))
    exit 0
} catch {
    Write-Classification (New-FullClassification "classifier failure (fail-closed): $($_.Exception.Message)")
    exit 0
}
