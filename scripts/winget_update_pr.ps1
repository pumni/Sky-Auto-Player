param (
    [Parameter(Mandatory=$true)]
    [string]$Version,
    
    [Parameter(Mandatory=$false)]
    [string]$GitHubToken
)

# Requires wingetcreate: winget install wingetcreate

$ErrorActionPreference = "Stop"

$ZipUrl = "https://github.com/pumni/Sky-Player/releases/download/v$Version/Sky-Player-v$Version.zip"

Write-Host "Triggering wingetcreate for pumni.SkyPlayer version $Version..."

if ([string]::IsNullOrWhiteSpace($GitHubToken)) {
    Write-Host "No GitHub token provided. Running interactively..."
    wingetcreate update pumni.SkyPlayer -v $Version -u $ZipUrl
} else {
    Write-Host "Running with provided GitHub token..."
    wingetcreate update pumni.SkyPlayer -v $Version -u $ZipUrl -t $GitHubToken
}

if ($LASTEXITCODE -ne 0) {
    Write-Error "wingetcreate failed."
    exit 1
}

Write-Host "Manifest PR submitted successfully (or local update complete)."
