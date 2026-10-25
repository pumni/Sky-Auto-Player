# Secure operator provisioning adapter for the Rust-owned Windows Credential Manager entry.
# The prompt is hidden and plaintext crosses only the child stdin pipe, never argv or output.

[CmdletBinding()]
param(
    # Internal regression seam accepts SecureString only; there is no plaintext password parameter.
    [Parameter(Mandatory = $false)]
    [System.Security.SecureString]$TestSecureString,

    # Isolated regression-test target; Rust validates its fixed test namespace.
    [Parameter(Mandatory = $false)]
    [string]$InternalTestTarget
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if (-not [System.OperatingSystem]::IsWindows()) {
    Write-Host "[FAIL] Windows Credential Manager is only supported on Windows"
    exit 1
}

$bstr = [IntPtr]::Zero
$passwordValue = $null
$process = $null
try {
    $secureInput = if ($null -ne $TestSecureString) {
        $TestSecureString
    } else {
        Read-Host -Prompt "Enter V4 updater production private key passphrase" -AsSecureString
    }
    if ($null -eq $secureInput -or $secureInput.Length -eq 0) {
        throw "Passphrase input cannot be empty"
    }

    $bstr = [System.Runtime.InteropServices.Marshal]::SecureStringToBSTR($secureInput)
    $passwordValue = [System.Runtime.InteropServices.Marshal]::PtrToStringBSTR($bstr)
    if ([string]::IsNullOrEmpty($passwordValue)) {
        throw "Passphrase input cannot be empty"
    }

    $repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = "cargo"
    $startInfo.WorkingDirectory = $repoRoot
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardInput = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $startInfo.StandardInputEncoding = [System.Text.UTF8Encoding]::new($false)
    foreach ($argument in @("xtask", "credential", "write-stdin")) {
        [void]$startInfo.ArgumentList.Add($argument)
    }
    if (-not [string]::IsNullOrWhiteSpace($InternalTestTarget)) {
        [void]$startInfo.ArgumentList.Add("--test-target")
        [void]$startInfo.ArgumentList.Add($InternalTestTarget)
    }

    $process = [System.Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    if (-not $process.Start()) {
        throw "Rust credential owner did not start"
    }
    $stdoutTask = $process.StandardOutput.ReadToEndAsync()
    $stderrTask = $process.StandardError.ReadToEndAsync()
    try {
        $process.StandardInput.Write($passwordValue)
        $process.StandardInput.Flush()
    } finally {
        $process.StandardInput.Close()
    }
    $process.WaitForExit()
    $stdout = $stdoutTask.GetAwaiter().GetResult()
    $stderr = $stderrTask.GetAwaiter().GetResult()
    if ($process.ExitCode -ne 0) {
        $safeOutput = ($stdout + $stderr).Replace($passwordValue, "[REDACTED]").Trim()
        if ([string]::IsNullOrWhiteSpace($safeOutput)) {
            throw "Rust credential owner failed with exit code $($process.ExitCode)"
        }
        throw "Rust credential owner failed: $safeOutput"
    }
    Write-Output $stdout.TrimEnd()
} catch {
    $safeError = $_.Exception.Message
    if (-not [string]::IsNullOrEmpty($passwordValue)) {
        $safeError = $safeError.Replace($passwordValue, "[REDACTED]")
    }
    Write-Host "[FAIL] Failed to provision session credential: $safeError"
    exit 1
} finally {
    if ($null -ne $process) {
        $process.Dispose()
    }
    if ($bstr -ne [IntPtr]::Zero) {
        [System.Runtime.InteropServices.Marshal]::ZeroFreeBSTR($bstr)
    }
    $passwordValue = $null
    Remove-Variable passwordValue -ErrorAction SilentlyContinue
}
