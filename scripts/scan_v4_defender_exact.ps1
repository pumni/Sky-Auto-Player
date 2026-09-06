[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$Artifact,

    [Parameter(Mandatory = $true)]
    [string]$Evidence
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Fail([string]$Message) {
    throw "V4 exact Defender scan failed closed: $Message"
}

$resolvedArtifact = (Resolve-Path -LiteralPath $Artifact -ErrorAction Stop).Path
$artifactItem = Get-Item -LiteralPath $resolvedArtifact -ErrorAction Stop
if ($artifactItem.PSIsContainer -or $artifactItem.Extension.ToLowerInvariant() -ne ".exe") {
    Fail "the exact downloaded artifact must be one regular installer PE"
}

$statusCommand = Get-Command Get-MpComputerStatus -ErrorAction SilentlyContinue
$scanCommand = Get-Command Start-MpScan -ErrorAction SilentlyContinue
$detectionsCommand = Get-Command Get-MpThreatDetection -ErrorAction SilentlyContinue
if ($null -eq $statusCommand -or $null -eq $scanCommand -or $null -eq $detectionsCommand) {
    Fail "Windows Defender status, custom-scan, and detection cmdlets are required"
}

try {
    $status = & $statusCommand.Name -ErrorAction Stop
    if (-not [bool]$status.AntivirusEnabled -or -not [bool]$status.RealTimeProtectionEnabled) {
        Fail "Windows Defender antivirus and real-time protection must be enabled"
    }

    $beforeHash = (Get-FileHash -LiteralPath $resolvedArtifact -Algorithm SHA256).Hash.ToLowerInvariant()
    $scanStarted = [DateTime]::UtcNow
    & $scanCommand.Name -ScanType CustomScan -ScanPath $resolvedArtifact -ErrorAction Stop | Out-Null
    $scanCompleted = [DateTime]::UtcNow
    if (-not (Test-Path -LiteralPath $resolvedArtifact -PathType Leaf)) {
        Fail "Defender removed or quarantined the exact downloaded installer"
    }
    $afterHash = (Get-FileHash -LiteralPath $resolvedArtifact -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($afterHash -ne $beforeHash) {
        Fail "the exact downloaded installer changed during Defender scanning"
    }

    $artifactFullPath = [IO.Path]::GetFullPath($resolvedArtifact)
    $detections = @(
        & $detectionsCommand.Name -ErrorAction Stop |
            Where-Object {
                $_.InitialDetectionTime -and
                $_.InitialDetectionTime.ToUniversalTime() -ge $scanStarted.AddSeconds(-2) -and
                (@($_.Resources | ForEach-Object { [string]$_ }) -join "`n") -match [regex]::Escape($artifactFullPath)
            } |
            ForEach-Object {
                [ordered]@{
                    initial_detection_time = $_.InitialDetectionTime.ToUniversalTime().ToString("o")
                    threat_id = $_.ThreatID
                    threat_name = $_.ThreatName
                    action_success = $_.ActionSuccess
                    resource_count = @($_.Resources).Count
                }
            }
    )
    if ($detections.Count -ne 0) {
        Fail "Defender detected a threat in the exact downloaded installer"
    }

    $parent = Split-Path -Parent $Evidence
    if ([string]::IsNullOrWhiteSpace($parent)) { Fail "Defender evidence requires a parent directory" }
    New-Item -ItemType Directory -Path $parent -Force | Out-Null
    [ordered]@{
        schema_version = 1
        evidence_type = "v4-defender-exact-artifact-scan"
        artifact = $artifactItem.Name
        artifact_size = [int64]$artifactItem.Length
        artifact_sha256 = $beforeHash
        scan_performed = $true
        scan_type = "CustomScan"
        scan_started_utc = $scanStarted.ToString("o")
        scan_completed_utc = $scanCompleted.ToString("o")
        detection_result = "none"
        detection_count = 0
        detections = @()
        antivirus_enabled = [bool]$status.AntivirusEnabled
        realtime_protection_enabled = [bool]$status.RealTimeProtectionEnabled
        unavailable = $false
    } | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath $Evidence -Encoding utf8
    Write-Host "V4 exact Defender scan: PASS (artifact=$($artifactItem.Name); detection_result=none)"
}
catch {
    if ($_.Exception.Message -like "V4 exact Defender scan failed closed:*") { throw }
    Fail "status, custom scan, or detection query was unavailable"
}
