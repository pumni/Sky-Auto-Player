[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateSet("Create", "Validate")]
    [string]$Mode,

    [string]$BundleDir,
    [string]$OutputRoot,
    [string]$BridgeRoot,
    [Parameter(Mandatory = $true)]
    [string]$SourceSha,
    [string]$Version,
    [string]$SentinelId,
    [string]$SentinelContentSha256,
    [string]$RepositoryRoot = (Get-Location).Path
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$semVerPattern = '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$'
$shaPattern = '^[0-9a-fA-F]{64}$'
$commitPattern = '^[0-9a-fA-F]{40}$'

function Fail([string]$Message) {
    throw "CI updater-bridge contract failed: $Message"
}

function Assert-CommitSha([string]$Value, [string]$Name) {
    if ([string]::IsNullOrWhiteSpace($Value) -or $Value -notmatch $commitPattern -or $Value -match '^0{40}$') {
        Fail "$Name must be a non-zero 40-character commit SHA"
    }
    return $Value.ToLowerInvariant()
}

function Assert-SafeFileName([string]$Name, [string]$Field) {
    if ([string]::IsNullOrWhiteSpace($Name) -or $Name.Length -gt 260 -or
        [IO.Path]::IsPathRooted($Name) -or [IO.Path]::GetFileName($Name) -cne $Name -or
        $Name.Contains("..") -or $Name.Contains([char]0)) {
        Fail "$Field is not a safe artifact-relative filename: $Name"
    }
}

function Get-DirectFiles([string]$Root) {
    if (-not (Test-Path -LiteralPath $Root -PathType Container)) {
        Fail "bridge artifact directory is missing: $Root"
    }
    $items = @(Get-ChildItem -LiteralPath $Root -Force)
    foreach ($item in $items) {
        if ($item.PSIsContainer -or (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0)) {
            Fail "bridge artifact contains an unexpected directory or reparse point: $($item.Name)"
        }
    }
    return @($items | Where-Object { -not $_.PSIsContainer })
}

function Get-Sha256([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        Fail "bridge file is missing: $Path"
    }
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Assert-Sentinel([string]$Value, [string]$Name) {
    if ([string]::IsNullOrWhiteSpace($Value) -or $Value.Length -gt 256 -or
        $Value.Contains([char]0) -or $Value -match '[\\/]') {
        Fail "$Name is not a bounded stable sentinel ID"
    }
}

function Read-BridgeContract {
    param(
        [Parameter(Mandatory = $true)] [string]$Root,
        [Parameter(Mandatory = $true)] [string]$ExpectedSourceSha,
        [string]$ExpectedVersion,
        [string]$ExpectedSentinelId,
        [string]$ExpectedSentinelContentSha256
    )

    $rootPath = (Resolve-Path -LiteralPath $Root -ErrorAction Stop).Path
    $files = Get-DirectFiles $rootPath
    if ($files.Count -ne 2) {
        Fail "bridge artifact must contain exactly two files; found $($files.Count)"
    }
    if (@($files | Where-Object { $_.Name -match 'PRIVATE KEY|SECRET KEY|\.sig$|\.pub$|\.key$' }).Count -ne 0) {
        Fail "bridge artifact contains private-key material or a detached signature"
    }
    $metadataPath = Join-Path $rootPath "bridge.json"
    if (-not (Test-Path -LiteralPath $metadataPath -PathType Leaf)) {
        Fail "bridge.json is missing"
    }

    try {
        $metadata = Get-Content -LiteralPath $metadataPath -Raw | ConvertFrom-Json
    } catch {
        Fail "bridge.json is not valid JSON: $($_.Exception.Message)"
    }
    $requiredFields = @(
        "schema_version", "source_sha", "version", "installer", "installer_sha256",
        "sentinel_id", "sentinel_content_sha256"
    )
    $observedFields = @($metadata.PSObject.Properties.Name)
    if (($observedFields -join "|") -cne ($requiredFields -join "|")) {
        Fail "bridge.json fields are not the locked contract: $($observedFields -join ', ')"
    }
    if ([int]$metadata.schema_version -ne 1) {
        Fail "unsupported bridge contract schema: $($metadata.schema_version)"
    }
    $expectedSource = Assert-CommitSha $ExpectedSourceSha "expected source SHA"
    if ([string]$metadata.source_sha -ine $expectedSource) {
        Fail "bridge source SHA does not match the checked-out source"
    }
    if ([string]$metadata.version -notmatch $semVerPattern) {
        Fail "bridge version is not canonical SemVer: $($metadata.version)"
    }
    if (-not [string]::IsNullOrWhiteSpace($ExpectedVersion) -and [string]$metadata.version -cne $ExpectedVersion) {
        Fail "bridge version does not match the producer/consumer binding"
    }
    Assert-SafeFileName ([string]$metadata.installer) "installer"
    if ([string]$metadata.installer -notmatch '-setup\.exe$') {
        Fail "bridge installer is not an NSIS setup executable"
    }
    if ([string]$metadata.installer_sha256 -notmatch $shaPattern) {
        Fail "bridge installer_sha256 is not a SHA-256 digest"
    }
    Assert-Sentinel ([string]$metadata.sentinel_id) "sentinel_id"
    if ([string]$metadata.sentinel_content_sha256 -notmatch $shaPattern) {
        Fail "bridge sentinel_content_sha256 is not a SHA-256 digest"
    }
    if (-not [string]::IsNullOrWhiteSpace($ExpectedSentinelId) -and [string]$metadata.sentinel_id -cne $ExpectedSentinelId) {
        Fail "bridge sentinel ID does not match the producer/consumer binding"
    }
    if (-not [string]::IsNullOrWhiteSpace($ExpectedSentinelContentSha256) -and
        [string]$metadata.sentinel_content_sha256 -ine $ExpectedSentinelContentSha256) {
        Fail "bridge sentinel content SHA does not match the producer/consumer binding"
    }

    $installerPath = Join-Path $rootPath ([string]$metadata.installer)
    if (-not (Test-Path -LiteralPath $installerPath -PathType Leaf)) {
        Fail "bridge.json references a missing installer: $installerPath"
    }
    $expectedNames = @("bridge.json", [string]$metadata.installer) | Sort-Object
    $observedNames = @($files | ForEach-Object { $_.Name } | Sort-Object)
    if (($observedNames -join "|") -cne ($expectedNames -join "|")) {
        Fail "bridge artifact contains unexpected files: $($observedNames -join ', ')"
    }
    $installerSha = Get-Sha256 $installerPath
    if ($installerSha -ine [string]$metadata.installer_sha256) {
        Fail "bridge installer SHA-256 does not match bridge.json"
    }

    return [pscustomobject]@{
        Root = $rootPath
        MetadataPath = $metadataPath
        InstallerPath = $installerPath
        SourceSha = ([string]$metadata.source_sha).ToLowerInvariant()
        Version = [string]$metadata.version
        InstallerSha256 = $installerSha
        SentinelId = [string]$metadata.sentinel_id
        SentinelContentSha256 = ([string]$metadata.sentinel_content_sha256).ToLowerInvariant()
    }
}

function New-BridgeContract {
    if ([string]::IsNullOrWhiteSpace($BundleDir) -or [string]::IsNullOrWhiteSpace($OutputRoot) -or
        [string]::IsNullOrWhiteSpace($Version) -or [string]::IsNullOrWhiteSpace($SentinelId) -or
        [string]::IsNullOrWhiteSpace($SentinelContentSha256)) {
        Fail "Create mode requires BundleDir, OutputRoot, Version, SentinelId, and SentinelContentSha256"
    }
    $sourceSha = Assert-CommitSha $SourceSha "source SHA"
    if ($Version -notmatch $semVerPattern) { Fail "bridge version is not canonical SemVer: $Version" }
    Assert-Sentinel $SentinelId "sentinel ID"
    if ($SentinelContentSha256 -notmatch $shaPattern) { Fail "sentinel content SHA is not a SHA-256 digest" }
    $bundlePath = (Resolve-Path -LiteralPath $BundleDir -ErrorAction Stop).Path
    $bundleFiles = Get-DirectFiles $bundlePath
    $installers = @($bundleFiles | Where-Object { $_.Name -match '-setup\.exe$' })
    if ($installers.Count -ne 1) { Fail "bridge build bundle must contain exactly one NSIS installer" }
    $installer = $installers[0]
    if (Test-Path -LiteralPath $OutputRoot) { Fail "bridge output directory already exists: $OutputRoot" }
    New-Item -ItemType Directory -Path $OutputRoot -Force | Out-Null
    Copy-Item -LiteralPath $installer.FullName -Destination (Join-Path $OutputRoot $installer.Name)
    $metadata = [ordered]@{
        schema_version = 1
        source_sha = $sourceSha
        version = $Version
        installer = $installer.Name
        installer_sha256 = Get-Sha256 (Join-Path $OutputRoot $installer.Name)
        sentinel_id = $SentinelId
        sentinel_content_sha256 = $SentinelContentSha256.ToLowerInvariant()
    }
    [IO.File]::WriteAllText(
        (Join-Path $OutputRoot "bridge.json"),
        ($metadata | ConvertTo-Json -Compress),
        [Text.UTF8Encoding]::new($false))
    return Read-BridgeContract $OutputRoot $sourceSha $Version $SentinelId $SentinelContentSha256
}

if ($Mode -eq "Create") {
    $contract = New-BridgeContract
} else {
    if ([string]::IsNullOrWhiteSpace($BridgeRoot)) { Fail "Validate mode requires BridgeRoot" }
    $contract = Read-BridgeContract $BridgeRoot $SourceSha $Version $SentinelId $SentinelContentSha256
}

Write-Output "CI updater-bridge contract: PASS (source=$($contract.SourceSha); version=$($contract.Version); installer=$($contract.InstallerSha256); sentinel=$($contract.SentinelId); sentinel_sha256=$($contract.SentinelContentSha256))"
