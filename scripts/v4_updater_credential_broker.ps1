# Narrow PowerShell command adapter for the Rust-owned V4 updater credential.
# Credential storage, decoding, lifecycle and cleanup are implemented by cargo xtask.

[CmdletBinding()]
param(
    [Parameter(Mandatory = $false)]
    [ValidateSet("Read", "Delete", "Test")]
    [string]$Action,

    # Isolated regression-test target. xtask accepts only its fixed test namespace.
    [Parameter(Mandatory = $false)]
    [string]$BrokerInternalTestTarget
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if (-not [System.OperatingSystem]::IsWindows()) {
    throw "V4 updater credential commands require Windows"
}

function Invoke-V4UpdaterCredentialCommand {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Command,
        [Parameter(Mandatory = $false)]
        [string]$TestTarget
    )

    $repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
    $arguments = @("xtask", "credential", $Command)
    if (-not [string]::IsNullOrWhiteSpace($TestTarget)) {
        $arguments += @("--test-target", $TestTarget)
    }
    Push-Location $repoRoot
    try {
        $output = (& cargo @arguments 2>&1 | Out-String)
        $exitCode = $LASTEXITCODE
    } finally {
        Pop-Location
    }
    if ($exitCode -ne 0) {
        $safeOutput = $output
        if (-not [string]::IsNullOrWhiteSpace($safeOutput)) {
            throw "Rust credential command failed: $($safeOutput.Trim())"
        }
        throw "Rust credential command failed with exit code $exitCode"
    }
    return $output.TrimEnd()
}

function Remove-V4UpdaterProductionCredential {
    [CmdletBinding()]
    param([string]$InternalTestTarget)
    Invoke-V4UpdaterCredentialCommand -Command "delete" -TestTarget $InternalTestTarget | Out-Null
}

function Test-V4UpdaterProductionCredential {
    [CmdletBinding()]
    param([string]$InternalTestTarget)
    $output = Invoke-V4UpdaterCredentialCommand -Command "status" -TestTarget $InternalTestTarget
    return $output -match "PRESENT_NONEMPTY"
}

function Test-V4UpdaterCredentialRead {
    [CmdletBinding()]
    param([string]$InternalTestTarget)
    Invoke-V4UpdaterCredentialCommand -Command "read-check" -TestTarget $InternalTestTarget | Out-Null
}

if ($PSCmdlet.MyInvocation.BoundParameters.ContainsKey("Action")) {
    switch ($Action) {
        "Delete" {
            Remove-V4UpdaterProductionCredential -InternalTestTarget $BrokerInternalTestTarget
            Write-Host "[PASS] V4 updater session credential deleted"
        }
        "Test" {
            if (Test-V4UpdaterProductionCredential -InternalTestTarget $BrokerInternalTestTarget) {
                Write-Host "PRESENT_NONEMPTY"
            } else {
                Write-Host "ABSENT"
            }
        }
        "Read" {
            Test-V4UpdaterCredentialRead -InternalTestTarget $BrokerInternalTestTarget
            Write-Host "[PASS] V4 updater credential read successfully"
        }
    }
}
