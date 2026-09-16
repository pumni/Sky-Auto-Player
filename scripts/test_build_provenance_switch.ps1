param(
    [string]$RepositoryRoot = (Get-Location).Path,
    [string]$CommitA,
    [string]$CommitB
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$PSNativeCommandUseErrorActionPreference = $false

$repoRoot = (Resolve-Path -LiteralPath $RepositoryRoot -ErrorAction Stop).Path
$commitA = if ([string]::IsNullOrWhiteSpace($CommitA)) {
    (& git -C $repoRoot rev-parse --verify HEAD).Trim()
} else {
    (& git -C $repoRoot rev-parse --verify $CommitA).Trim()
}
if ($LASTEXITCODE -ne 0 -or $commitA -notmatch '^[0-9a-fA-F]{40}$') {
    throw "Commit A is not a valid commit SHA"
}

$commitB = if ([string]::IsNullOrWhiteSpace($CommitB)) {
    (& git -C $repoRoot rev-parse --verify "$commitA^1").Trim()
} else {
    (& git -C $repoRoot rev-parse --verify $CommitB).Trim()
}
if ($LASTEXITCODE -ne 0 -or $commitB -notmatch '^[0-9a-fA-F]{40}$') {
    throw "Commit B is not a valid commit SHA"
}
if ($commitA -ieq $commitB) {
    throw "Commit A and commit B must differ"
}

$acceptanceRoot = Join-Path ([IO.Path]::GetTempPath()) ("sky-build-provenance-" + [guid]::NewGuid().ToString("N"))
$worktreePath = Join-Path $acceptanceRoot "worktree"
$targetPath = Join-Path $worktreePath "rust/target"
$trackedEnvironment = @(
    "CARGO_TARGET_DIR",
    "TAURI_CONFIG",
    "SKY_CI_SOURCE_SHA",
    "GITHUB_SHA",
    "SKY_NATIVE_BUILD_COMMIT",
    "SKY_NATIVE_DIRTY_WORKTREE",
    "SKY_NATIVE_SOURCE_FINGERPRINT"
)
$previousEnvironment = @{}
foreach ($name in $trackedEnvironment) {
    $previousEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, "Process")
    Remove-Item -LiteralPath ("Env:" + $name) -ErrorAction SilentlyContinue
}

function Invoke-NativeBuild([string]$Commit, [string]$Label) {
    & git -C $worktreePath checkout --quiet --detach $Commit 2>$null
    if ($LASTEXITCODE -ne 0) { throw "Failed to check out $Label ($Commit)" }

    [Environment]::SetEnvironmentVariable("CARGO_TARGET_DIR", $targetPath, "Process")
    [Environment]::SetEnvironmentVariable("TAURI_CONFIG", '{"build":{"frontendDist":null}}', "Process")
    & cargo build `
        --manifest-path (Join-Path $worktreePath "rust/Cargo.toml") `
        -p sky_desktop_shell `
        --bin sky_desktop_shell `
        --no-default-features `
        --features desktop-runtime `
        --locked
    if ($LASTEXITCODE -ne 0) { throw "$Label native build failed" }

    $executable = Join-Path $targetPath "debug/sky_desktop_shell.exe"
    if (-not (Test-Path -LiteralPath $executable -PathType Leaf)) {
        throw "$Label native executable is missing: $executable"
    }
    $metadataJson = (& $executable --selftest-build-info | Out-String).Trim()
    if ($LASTEXITCODE -ne 0) { throw "$Label build-info self-test failed" }
    $metadata = $metadataJson | ConvertFrom-Json
    $embedded = [string]$metadata.native_build_commit
    $embeddedBase = $embedded -replace '-dirty$', ''
    if ($embeddedBase -ine $Commit) {
        throw "$Label embedded SHA mismatch: expected=$Commit actual=$embedded"
    }
    Write-Host "${Label}: PASS (HEAD=$Commit, embedded_native_build_commit=$embedded)"
}

try {
    New-Item -ItemType Directory -Path $acceptanceRoot -Force | Out-Null
    & git -C $repoRoot worktree add --quiet --detach $worktreePath $commitA 2>$null
    if ($LASTEXITCODE -ne 0) { throw "Failed to create the temporary acceptance worktree" }

    Invoke-NativeBuild $commitA "commit A"
    Invoke-NativeBuild $commitB "commit B"
    Write-Host "build provenance switch acceptance: PASS (same target, no cargo clean)"
} finally {
    foreach ($name in $trackedEnvironment) {
        $value = $previousEnvironment[$name]
        if ($null -eq $value) {
            Remove-Item -LiteralPath ("Env:" + $name) -ErrorAction SilentlyContinue
        } else {
            [Environment]::SetEnvironmentVariable($name, $value, "Process")
        }
    }
    $worktreeRemoved = $true
    if (Test-Path -LiteralPath $worktreePath) {
        & git -C $repoRoot worktree remove --force $worktreePath
        $worktreeRemoved = $LASTEXITCODE -eq 0
        if (-not $worktreeRemoved) {
            Write-Warning "Temporary acceptance worktree could not be removed: $worktreePath"
        }
    }
    if ($worktreeRemoved -and (Test-Path -LiteralPath $acceptanceRoot)) {
        Remove-Item -LiteralPath $acceptanceRoot -Recurse -Force -ErrorAction SilentlyContinue
    }
}
