[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$InstallRoot,
    [Parameter(Mandatory = $true)]
    [long]$ExpectedSize,
    [Parameter(Mandatory = $true)]
    [string]$ExpectedSha256,
    [string]$Evidence
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Fail([string]$Message) {
    throw "Installed calibration verification failed: $Message"
}

function Get-PeMachine([string]$Path) {
    $stream = [IO.File]::Open($Path, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    $reader = [IO.BinaryReader]::new($stream)
    try {
        if ($stream.Length -lt 0x40) { Fail "installed calibration executable is not a complete PE file" }
        $stream.Position = 0
        if ([Text.Encoding]::ASCII.GetString($reader.ReadBytes(2)) -cne "MZ") {
            Fail "installed calibration executable is missing the DOS header"
        }
        $stream.Position = 0x3c
        $peOffset = $reader.ReadInt32()
        if ($peOffset -lt 0x40 -or $peOffset -gt ($stream.Length - 6)) {
            Fail "installed calibration executable has an invalid PE header offset"
        }
        $stream.Position = $peOffset
        $signature = @($reader.ReadByte(), $reader.ReadByte(), $reader.ReadByte(), $reader.ReadByte())
        if ($signature[0] -ne 0x50 -or $signature[1] -ne 0x45 -or
            $signature[2] -ne 0 -or $signature[3] -ne 0) {
            Fail "installed calibration executable is missing the PE signature"
        }
        return [int]$reader.ReadUInt16()
    } finally {
        $reader.Dispose()
        $stream.Dispose()
    }
}

if ($ExpectedSize -le 0) { Fail "expected calibration binary size must be positive" }
if ($ExpectedSha256 -notmatch '^[0-9a-fA-F]{64}$') { Fail "expected calibration SHA-256 is invalid" }
$root = (Resolve-Path -LiteralPath $InstallRoot -ErrorAction Stop).Path
if (-not (Test-Path -LiteralPath $root -PathType Container)) { Fail "install root is not a directory" }
$binaryPath = Join-Path $root "native_calibration.exe"
if (-not (Test-Path -LiteralPath $binaryPath -PathType Leaf)) { Fail "installed native_calibration.exe is missing" }
$binary = Get-Item -LiteralPath $binaryPath -ErrorAction Stop
if ($binary.PSIsContainer -or (($binary.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0)) {
    Fail "installed native_calibration.exe must be a regular file"
}
if ([long]$binary.Length -ne $ExpectedSize) {
    Fail "installed calibration size mismatch: expected=$ExpectedSize observed=$($binary.Length)"
}
$sha256 = (Get-FileHash -LiteralPath $binaryPath -Algorithm SHA256).Hash.ToLowerInvariant()
if ($sha256 -cne $ExpectedSha256.ToLowerInvariant()) {
    Fail "installed calibration SHA-256 mismatch: expected=$($ExpectedSha256.ToLowerInvariant()) observed=$sha256"
}
$machine = Get-PeMachine $binaryPath
if ($machine -ne 0x8664) { Fail "installed calibration executable is not x86_64 (PE machine 0x$('{0:x4}' -f $machine))" }

$startInfo = [Diagnostics.ProcessStartInfo]::new()
$startInfo.FileName = $binaryPath
$startInfo.UseShellExecute = $false
$startInfo.CreateNoWindow = $true
$startInfo.RedirectStandardOutput = $true
$startInfo.RedirectStandardError = $true
$startInfo.ArgumentList.Add("--help")
$process = [Diagnostics.Process]::new()
$process.StartInfo = $startInfo
try {
    if (-not $process.Start()) { Fail "installed calibration help process did not start" }
    $stdoutTask = $process.StandardOutput.ReadToEndAsync()
    $stderrTask = $process.StandardError.ReadToEndAsync()
    if (-not $process.WaitForExit(5000)) {
        try { $process.Kill($true) } catch { }
        [void]$process.WaitForExit(1000)
        Fail "installed calibration --help exceeded the 5-second smoke bound"
    }
    $helpOutput = $stdoutTask.GetAwaiter().GetResult() + [Environment]::NewLine + $stderrTask.GetAwaiter().GetResult()
    if ($process.ExitCode -ne 0) { Fail "installed calibration --help exited with $($process.ExitCode)" }
    if ($helpOutput -notmatch '(?im)^\s*usage:\s*native_calibration\b') {
        Fail "installed calibration --help did not print its usage"
    }
} finally {
    $process.Dispose()
}

$result = [ordered]@{
    schema_version = 1
    evidence_type = "installed-native-calibration"
    verified = $true
    path = "native_calibration.exe"
    size = [long]$binary.Length
    sha256 = $sha256
    architecture = "x86_64"
    pe_machine = "0x8664"
    help_exit_code = 0
    help_usage_verified = $true
}
if (-not [string]::IsNullOrWhiteSpace($Evidence)) {
    $evidenceDirectory = Split-Path -Parent $Evidence
    if (-not [string]::IsNullOrWhiteSpace($evidenceDirectory)) {
        New-Item -ItemType Directory -Path $evidenceDirectory -Force | Out-Null
    }
    $result | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $Evidence -Encoding utf8
}
Write-Output "Installed calibration verification: PASS (size=$($binary.Length); sha256=$sha256; architecture=x86_64; help=PASS)"
