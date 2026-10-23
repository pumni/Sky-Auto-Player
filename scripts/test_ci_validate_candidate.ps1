[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$validator = Join-Path $PSScriptRoot "ci_validate_candidate.ps1"
$tempRoot = Join-Path ([IO.Path]::GetTempPath()) ("sky-ci-candidate-" + [guid]::NewGuid().ToString("N"))
$sourceSha = "1234567890abcdef1234567890abcdef12345678"

function Fail([string]$Message) { throw "CI candidate contract self-test failed: $Message" }

function Invoke-Validator([string[]]$Arguments) {
    $output = @(& pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $validator @Arguments 2>&1)
    return [pscustomobject]@{ ExitCode = $LASTEXITCODE; Output = ($output -join "`n") }
}

function Assert-Fails([string]$Name, [string[]]$Arguments) {
    $result = Invoke-Validator $Arguments
    if ($result.ExitCode -eq 0) { Fail "$Name unexpectedly passed" }
}

try {
    New-Item -ItemType Directory -Path $tempRoot -Force | Out-Null
    $bundle = Join-Path $tempRoot "bundle"
    $candidate = Join-Path $tempRoot "candidate"
    New-Item -ItemType Directory -Path $bundle -Force | Out-Null
    $cargoPath = Join-Path $repoRoot "desktop/src-tauri/Cargo.toml"
    $cargo = Get-Content -LiteralPath $cargoPath -Raw
    $versionMatch = [regex]::Match($cargo, '(?m)^version\s*=\s*"([^"]+)"')
    if (-not $versionMatch.Success) { Fail "canonical package version is missing from $cargoPath" }
    $candidateVersion = $versionMatch.Groups[1].Value
    $installerName = "Sky Auto Player_${candidateVersion}_x64-setup.exe"
    [IO.File]::WriteAllBytes((Join-Path $bundle $installerName), [byte[]](0..31))
    [IO.File]::WriteAllText((Join-Path $bundle "$installerName.sig"), "official test updater signature`n")
    $publicKey = Join-Path $tempRoot "test-public-key.pub"
    [IO.File]::WriteAllText($publicKey, "dGVzdC1wdWJsaWMta2V5")
    $calibrationBinary = Join-Path $tempRoot "native_calibration.exe"
    $calibrationBytes = [byte[]]::new(0x86)
    $calibrationBytes[0] = 0x4d
    $calibrationBytes[1] = 0x5a
    [BitConverter]::GetBytes([int]0x80).CopyTo($calibrationBytes, 0x3c)
    $calibrationBytes[0x80] = 0x50
    $calibrationBytes[0x81] = 0x45
    [BitConverter]::GetBytes([UInt16]0x8664).CopyTo($calibrationBytes, 0x84)
    [IO.File]::WriteAllBytes($calibrationBinary, $calibrationBytes)

    $create = Invoke-Validator @(
        "-Mode", "Create", "-BundleDir", $bundle, "-PublicKeyPath", $publicKey, "-CalibrationBinaryPath", $calibrationBinary,
        "-OutputRoot", $candidate, "-SourceSha", $sourceSha, "-RepositoryRoot", $repoRoot
    )
    if ($create.ExitCode -ne 0) { Fail "Create mode failed: $($create.Output)" }
    $validate = Invoke-Validator @(
        "-Mode", "Validate", "-CandidateRoot", $candidate, "-SourceSha", $sourceSha,
        "-RepositoryRoot", $repoRoot
    )
    if ($validate.ExitCode -ne 0) { Fail "Validate mode failed: $($validate.Output)" }
    $metadata = Get-Content -LiteralPath (Join-Path $candidate "candidate.json") -Raw | ConvertFrom-Json
    if ([int]$metadata.schema_version -ne 2 -or [string]$metadata.version -ne $candidateVersion -or
        [string]$metadata.source_sha -ne $sourceSha -or
        [long]$metadata.calibration_binary_size -ne (Get-Item -LiteralPath $calibrationBinary).Length -or
        [string]$metadata.calibration_binary_sha256 -cne (Get-FileHash -LiteralPath $calibrationBinary -Algorithm SHA256).Hash.ToLowerInvariant() -or
        [string]$metadata.calibration_binary_architecture -cne "x86_64") {
        Fail "created candidate metadata did not bind source, version, and calibration binary identity"
    }
    if (@(Get-ChildItem -LiteralPath $candidate -File).Count -ne 4) {
        Fail "created candidate artifact did not contain exactly four files"
    }

    $installerPath = Join-Path $candidate $installerName
    Add-Content -LiteralPath $installerPath -Value "tampered" -Encoding utf8
    Assert-Fails "installer tamper" @(
        "-Mode", "Validate", "-CandidateRoot", $candidate, "-SourceSha", $sourceSha,
        "-RepositoryRoot", $repoRoot
    )
    [IO.File]::WriteAllBytes($installerPath, [byte[]](0..31))

    Assert-Fails "source mismatch" @(
        "-Mode", "Validate", "-CandidateRoot", $candidate,
        "-SourceSha", ("a" * 40), "-RepositoryRoot", $repoRoot
    )
    [IO.File]::WriteAllText((Join-Path $candidate "updater-public-key.pub"), "PRIVATE KEY")
    Assert-Fails "private key material" @(
        "-Mode", "Validate", "-CandidateRoot", $candidate, "-SourceSha", $sourceSha,
        "-RepositoryRoot", $repoRoot
    )

    Write-Output "CI candidate contract self-tests: PASS"
} finally {
    if (Test-Path -LiteralPath $tempRoot) {
        Remove-Item -LiteralPath $tempRoot -Recurse -Force -ErrorAction SilentlyContinue
    }
}
