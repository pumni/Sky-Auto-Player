[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$verifier = Join-Path $PSScriptRoot "verify_installed_calibration.ps1"
$tempRoot = Join-Path ([IO.Path]::GetTempPath()) ("sky-installed-calibration-" + [guid]::NewGuid().ToString("N"))

function Fail([string]$Message) {
    throw "Installed calibration missing-binary self-test failed: $Message"
}

try {
    New-Item -ItemType Directory -Path $tempRoot -Force | Out-Null
    $output = @(& pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $verifier -InstallRoot $tempRoot -ExpectedSize 1 -ExpectedSha256 ("0" * 64) 2>&1)
    $exitCode = $LASTEXITCODE
    if ($exitCode -eq 0) { Fail "verifier accepted an install root without native_calibration.exe" }
    if (($output -join [Environment]::NewLine) -notmatch "native_calibration\.exe is missing") {
        Fail "missing-binary failure did not identify the absent executable: $($output -join ' ')"
    }
    Write-Output "Installed calibration missing-binary self-test: PASS"
} finally {
    if (Test-Path -LiteralPath $tempRoot) {
        Remove-Item -LiteralPath $tempRoot -Recurse -Force -ErrorAction SilentlyContinue
    }
}
