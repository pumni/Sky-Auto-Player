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
    $installerName = "Sky Auto Player_4.0.1_x64-setup.exe"
    [IO.File]::WriteAllBytes((Join-Path $bundle $installerName), [byte[]](0..31))
    [IO.File]::WriteAllText((Join-Path $bundle "$installerName.sig"), "official test updater signature`n")
    $publicKey = Join-Path $tempRoot "test-public-key.pub"
    [IO.File]::WriteAllText($publicKey, "dGVzdC1wdWJsaWMta2V5")

    $create = Invoke-Validator @(
        "-Mode", "Create", "-BundleDir", $bundle, "-PublicKeyPath", $publicKey,
        "-OutputRoot", $candidate, "-SourceSha", $sourceSha, "-RepositoryRoot", $repoRoot
    )
    if ($create.ExitCode -ne 0) { Fail "Create mode failed: $($create.Output)" }
    $validate = Invoke-Validator @(
        "-Mode", "Validate", "-CandidateRoot", $candidate, "-SourceSha", $sourceSha,
        "-RepositoryRoot", $repoRoot
    )
    if ($validate.ExitCode -ne 0) { Fail "Validate mode failed: $($validate.Output)" }
    $metadata = Get-Content -LiteralPath (Join-Path $candidate "candidate.json") -Raw | ConvertFrom-Json
    if ([string]$metadata.version -ne "4.0.1" -or [string]$metadata.source_sha -ne $sourceSha) {
        Fail "created candidate metadata did not bind version and source SHA"
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
