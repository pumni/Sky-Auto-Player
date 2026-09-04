$ErrorActionPreference = "Stop"

# This is deliberately read-only. It proves the public v3 discovery response
# still resolves to a v3 release while the dedicated v4 authority exists. It
# never creates, edits, publishes, or deletes a release.
$sourceRepository = "pumni/Sky-Auto-Player"
$authorityRepository = "pumni/Sky-Auto-Player-Releases"

function Get-GitHubJson([string]$Path) {
    $payload = & gh api $Path --header "Accept: application/vnd.github+json"
    if ($LASTEXITCODE -ne 0) {
        throw "GitHub read failed for $Path"
    }
    return ($payload -join "`n") | ConvertFrom-Json
}

$source = Get-GitHubJson "repos/$sourceRepository/releases/latest"
if ($source.url -notlike "https://api.github.com/repos/$sourceRepository/releases/*") {
    throw "Unexpected source repository in latest-release response: $($source.url)"
}
if ($source.draft -or $source.prerelease) {
    throw "The v3 latest release must be published and non-prerelease: $($source.tag_name)"
}
if ($source.tag_name -notmatch '^v(?<version>3\.\d+\.\d+)$') {
    throw "The source repository latest release is not a canonical v3 tag: $($source.tag_name)"
}

$version = $Matches.version
$assetNames = @($source.assets | ForEach-Object { $_.name })
$requiredV3Assets = @(
    "Sky-Auto-Player-v$version.zip",
    "Sky-Auto-Player-v$version.zip.sha256",
    "MANIFEST.json"
)
foreach ($asset in $requiredV3Assets) {
    if ($assetNames -notcontains $asset) {
        throw "The v3 latest release is missing its canonical asset: $asset"
    }
}
if ($assetNames | Where-Object { $_ -match '^Sky Auto Player_4(?:\.\d+|-)'} ) {
    throw "A canonical v4 Tauri artifact appeared in the v3 source release namespace"
}

$authority = Get-GitHubJson "repos/$authorityRepository"
if ($authority.full_name -ne $authorityRepository -or $authority.private) {
    throw "The dedicated v4 release authority is not the expected public repository"
}

@"
V4 release-authority acceptance: PASS
source_latest_tag=$($source.tag_name)
source_latest_release=$($source.html_url)
v4_authority=$($authority.html_url)
v3_canonical_assets=$($requiredV3Assets -join ',')
read_only=true
"@ | Write-Host
