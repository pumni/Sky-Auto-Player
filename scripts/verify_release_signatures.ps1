[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$ReleaseDirectory,
    [Parameter(Mandatory = $true)]
    [string]$ExpectedPublisherSubject
)

$ErrorActionPreference = "Stop"
$release = (Resolve-Path -LiteralPath $ReleaseDirectory).Path
if ([string]::IsNullOrWhiteSpace($ExpectedPublisherSubject)) {
    throw "Expected publisher subject is required."
}
$targets = @(
    (Join-Path $release "Sky-Auto-Player.exe"),
    (Join-Path $release "Sky-Auto-Player-Updater.exe"),
    (Join-Path $release "native_calibration.exe")
)
$pyds = @(Get-ChildItem (Join-Path $release "_internal") -Recurse -Filter "sky_player_rs*.pyd" -File |
    ForEach-Object { $_.FullName })
if ($pyds.Count -eq 0) {
    throw "Missing project-owned sky_player_rs*.pyd signing target."
}
$targets += $pyds
foreach ($target in $targets) {
    if (-not (Test-Path -LiteralPath $target -PathType Leaf)) {
        throw "Missing project-owned signing target."
    }
    $signature = Get-AuthenticodeSignature -LiteralPath $target
    if ($signature.Status -ne "Valid" -or $signature.SignerCertificate.Subject -ne $ExpectedPublisherSubject) {
        throw "Signature or publisher policy failed for a project-owned PE."
    }
}
Write-Host "Project-owned Authenticode signatures verified."
