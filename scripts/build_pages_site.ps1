param (
    [Parameter(Mandatory=$true)]
    [string]$Source,

    [Parameter(Mandatory=$true)]
    [string]$Destination
)

$ErrorActionPreference = 'Stop'

Write-Host "Building Pages artifact from '$Source' to '$Destination'..."

if (Test-Path $Destination) {
    Remove-Item -Recurse -Force $Destination
}
New-Item -ItemType Directory -Path $Destination | Out-Null

$requiredFiles = @(
    "index.html",
    "faq.html",
    "vi/index.html",
    "vi/faq.html",
    "sitemap.xml",
    "llms.txt"
)

# Copy required files
foreach ($file in $requiredFiles) {
    $srcFile = Join-Path $Source $file
    $destFile = Join-Path $Destination $file
    
    if (-not (Test-Path $srcFile)) {
        Write-Error "Required file missing: $srcFile"
        exit 1
    }

    $destDir = Split-Path $destFile -Parent
    if (-not (Test-Path $destDir)) {
        New-Item -ItemType Directory -Path $destDir | Out-Null
    }

    Copy-Item -Path $srcFile -Destination $destFile
}

# Copy assets directory
$srcAssets = Join-Path $Source "assets"
$destAssets = Join-Path $Destination "assets"
if (Test-Path $srcAssets) {
    Copy-Item -Path $srcAssets -Destination $destAssets -Recurse
} else {
    Write-Error "Required directory missing: $srcAssets"
    exit 1
}

# Copy google*.html
$googleFiles = Get-ChildItem -Path $Source -Filter "google*.html" -File
foreach ($gFile in $googleFiles) {
    Copy-Item -Path $gFile.FullName -Destination (Join-Path $Destination $gFile.Name)
}

# Create .nojekyll
$nojekyllPath = Join-Path $Destination ".nojekyll"
if (-not (Test-Path (Join-Path $Source ".nojekyll"))) {
    New-Item -ItemType File -Path $nojekyllPath | Out-Null
} else {
    Copy-Item -Path (Join-Path $Source ".nojekyll") -Destination $nojekyllPath
}

# Print manifest
Write-Host "`nArtifact Manifest:"
$manifest = Get-ChildItem -Path $Destination -Recurse -File | 
    Select-Object -ExpandProperty FullName | 
    ForEach-Object { $_.Substring((Convert-Path $Destination).Length + 1).Replace('\', '/') } | 
    Sort-Object

$manifest | ForEach-Object { Write-Host $_ }
Write-Host "`nBuild completed successfully."
