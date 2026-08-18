[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$OutputPath,
    [Parameter(Mandatory = $true)]
    [DateTime]$SinceUtc
)

$ErrorActionPreference = "Stop"

try {
    $status = Get-MpComputerStatus -ErrorAction Stop
    $preference = Get-MpPreference -ErrorAction Stop
    $rawExclusions = @($preference.ExclusionPath)
    if ($rawExclusions.Count -eq 1 -and [string]$rawExclusions[0] -match '^N/A:') {
        throw "Defender exclusions are not readable from the elevated helper"
    }

    $exclusions = @(
        $rawExclusions |
            ForEach-Object { [string]$_ }
    ) | Sort-Object
    $threats = @(
        Get-MpThreatDetection -ErrorAction Stop |
            Where-Object {
                $_.InitialDetectionTime -and
                $_.InitialDetectionTime.ToUniversalTime() -ge $SinceUtc.ToUniversalTime()
            } |
            ForEach-Object {
                [ordered]@{
                    initial_detection_time = $_.InitialDetectionTime
                    threat_id = $_.ThreatID
                    threat_name = $_.ThreatName
                    action_success = $_.ActionSuccess
                    resources = @($_.Resources | ForEach-Object { [string]$_ })
                }
            }
    )

    $parent = [IO.Path]::GetDirectoryName($OutputPath)
    if ([string]::IsNullOrWhiteSpace($parent)) {
        throw "Defender evidence output must have a parent directory"
    }
    New-Item -ItemType Directory -Path $parent -Force | Out-Null
    [ordered]@{
        antivirus_enabled = [bool]$status.AntivirusEnabled
        realtime_protection_enabled = [bool]$status.RealTimeProtectionEnabled
        exclusions = @($exclusions)
        threat_detections = @($threats)
        threat_detection_count = $threats.Count
        snapshot_elevated = $true
    } |
        ConvertTo-Json -Depth 12 |
        Set-Content -LiteralPath $OutputPath -Encoding utf8
}
catch {
    Write-Error $_
    exit 1
}
