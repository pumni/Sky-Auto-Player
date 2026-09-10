[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateSet("Create", "Validate")]
    [string]$Mode,

    [string]$BundleDir,
    [string]$PublicKeyPath,
    [string]$OutputRoot,
    [string]$CandidateRoot,
    [Parameter(Mandatory = $true)]
    [string]$SourceSha,
    [string]$RepositoryRoot = (Get-Location).Path
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$semVerPattern = '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$'
$shaPattern = '^[0-9a-fA-F]{64}$'
$commitPattern = '^[0-9a-fA-F]{40}$'
$publicKeyFileName = "updater-public-key.pub"

function Fail([string]$Message) {
    throw "CI candidate contract failed: $Message"
}

function Assert-CommitSha([string]$Value, [string]$Name) {
    if ($Value -notmatch $commitPattern -or $Value -match '^0{40}$') {
        Fail "$Name must be a non-zero 40-character commit SHA"
    }
    return $Value.ToLowerInvariant()
}

function Get-ProjectVersion([string]$Root) {
    $manifestPath = Join-Path $Root "desktop/src-tauri/Cargo.toml"
    if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
        Fail "canonical desktop manifest is missing: $manifestPath"
    }
    $source = Get-Content -LiteralPath $manifestPath -Raw
    $matches = [regex]::Matches($source, '(?m)^version\s*=\s*"([^"]+)"\s*$')
    if ($matches.Count -ne 1) {
        Fail "canonical desktop manifest must contain exactly one package version"
    }
    $version = $matches[0].Groups[1].Value
    if ($version -notmatch $semVerPattern) {
        Fail "canonical desktop package version is not SemVer: $version"
    }
    return $version
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
        Fail "candidate directory is missing: $Root"
    }
    $items = @(Get-ChildItem -LiteralPath $Root -Force)
    foreach ($item in $items) {
        if ($item.PSIsContainer -or (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0)) {
            Fail "candidate directory contains an unexpected directory or reparse point: $($item.Name)"
        }
    }
    return @($items | Where-Object { -not $_.PSIsContainer })
}

function Get-Sha256([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        Fail "candidate file is missing: $Path"
    }
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Validate-PublicKey([string]$Path) {
    $item = Get-Item -LiteralPath $Path -ErrorAction Stop
    if ($item.Length -le 0 -or $item.Length -gt 4096) {
        Fail "candidate public key is empty or unbounded"
    }
    $value = ([IO.File]::ReadAllText($Path)).Trim()
    if ([string]::IsNullOrWhiteSpace($value) -or $value -match 'PRIVATE KEY') {
        Fail "candidate public key is missing or contains private key material"
    }
    if ($value -notmatch '^[A-Za-z0-9+/=\s]+$') {
        Fail "candidate public key contains unexpected material"
    }
}

function Read-CandidateContract([string]$Root, [string]$ExpectedSourceSha, [string]$ExpectedVersion) {
    $rootPath = (Resolve-Path -LiteralPath $Root -ErrorAction Stop).Path
    $files = Get-DirectFiles $rootPath
    $metadataPath = Join-Path $rootPath "candidate.json"
    if (-not (Test-Path -LiteralPath $metadataPath -PathType Leaf)) {
        Fail "candidate.json is missing"
    }
    if ($files.Count -ne 4) {
        Fail "candidate artifact must contain exactly four files; found $($files.Count)"
    }

    $metadata = Get-Content -LiteralPath $metadataPath -Raw | ConvertFrom-Json
    $requiredFields = @(
        "schema_version", "source_sha", "version", "installer", "installer_sha256",
        "updater_signature", "updater_signature_sha256", "updater_public_key",
        "updater_public_key_sha256"
    )
    $observedFields = @($metadata.PSObject.Properties.Name)
    if (($observedFields -join "|") -cne ($requiredFields -join "|")) {
        Fail "candidate.json fields are not the locked contract: $($observedFields -join ', ')"
    }
    if ([int]$metadata.schema_version -ne 1) {
        Fail "unsupported candidate contract schema: $($metadata.schema_version)"
    }
    $expectedSource = Assert-CommitSha $ExpectedSourceSha "expected source SHA"
    if ([string]$metadata.source_sha -ine $expectedSource) {
        Fail "candidate source SHA does not match the checked-out source"
    }
    if ([string]$metadata.version -cne $ExpectedVersion -or [string]$metadata.version -notmatch $semVerPattern) {
        Fail "candidate version does not match the checked-out source version"
    }

    foreach ($field in @("installer", "updater_signature", "updater_public_key")) {
        Assert-SafeFileName ([string]$metadata.$field) $field
    }
    if ([string]$metadata.installer -notmatch '-setup\.exe$') {
        Fail "candidate installer is not an NSIS setup executable"
    }
    if ([string]$metadata.updater_signature -cne "$($metadata.installer).sig") {
        Fail "candidate updater signature is not the exact installer pair"
    }
    if ([string]$metadata.updater_public_key -cne $publicKeyFileName) {
        Fail "candidate public key filename is not the locked public-key filename"
    }
    foreach ($field in @("installer_sha256", "updater_signature_sha256", "updater_public_key_sha256")) {
        if ([string]$metadata.$field -notmatch $shaPattern) {
            Fail "$field is not a SHA-256 digest"
        }
    }

    $installerPath = Join-Path $rootPath ([string]$metadata.installer)
    $signaturePath = Join-Path $rootPath ([string]$metadata.updater_signature)
    $publicKeyPath = Join-Path $rootPath ([string]$metadata.updater_public_key)
    foreach ($path in @($installerPath, $signaturePath, $publicKeyPath)) {
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
            Fail "candidate.json references a missing artifact file: $path"
        }
    }
    $expectedNames = @("candidate.json", [string]$metadata.installer, [string]$metadata.updater_signature, $publicKeyFileName)
    $observedNames = @($files | ForEach-Object { $_.Name } | Sort-Object)
    $sortedExpectedNames = @($expectedNames | Sort-Object)
    if (($observedNames -join "|") -cne ($sortedExpectedNames -join "|")) {
        Fail "candidate artifact contains unexpected files: $($observedNames -join ', ')"
    }

    $installerSha = Get-Sha256 $installerPath
    $signatureSha = Get-Sha256 $signaturePath
    $publicKeySha = Get-Sha256 $publicKeyPath
    if ($installerSha -ine [string]$metadata.installer_sha256) { Fail "installer SHA-256 does not match candidate.json" }
    if ($signatureSha -ine [string]$metadata.updater_signature_sha256) { Fail "updater signature SHA-256 does not match candidate.json" }
    if ($publicKeySha -ine [string]$metadata.updater_public_key_sha256) { Fail "updater public-key SHA-256 does not match candidate.json" }
    Validate-PublicKey $publicKeyPath

    return [pscustomobject]@{
        Root = $rootPath
        MetadataPath = $metadataPath
        InstallerPath = $installerPath
        SignaturePath = $signaturePath
        PublicKeyPath = $publicKeyPath
        Version = [string]$metadata.version
        SourceSha = ([string]$metadata.source_sha).ToLowerInvariant()
        InstallerSha256 = $installerSha
        SignatureSha256 = $signatureSha
        PublicKeySha256 = $publicKeySha
    }
}

function New-CandidateContract {
    if ([string]::IsNullOrWhiteSpace($BundleDir) -or [string]::IsNullOrWhiteSpace($PublicKeyPath) -or
        [string]::IsNullOrWhiteSpace($OutputRoot)) {
        Fail "Create mode requires BundleDir, PublicKeyPath, and OutputRoot"
    }
    $sourceSha = Assert-CommitSha $SourceSha "source SHA"
    $version = Get-ProjectVersion $RepositoryRoot
    $bundlePath = (Resolve-Path -LiteralPath $BundleDir -ErrorAction Stop).Path
    $bundleFiles = Get-DirectFiles $bundlePath
    $installers = @($bundleFiles | Where-Object { $_.Name -match '-setup\.exe$' })
    if ($installers.Count -ne 1 -or $bundleFiles.Count -ne 2) {
        Fail "candidate bundle must contain exactly one installer and one signature"
    }
    $installer = $installers[0]
    $signature = Get-Item -LiteralPath (Join-Path $bundlePath "$($installer.Name).sig") -ErrorAction Stop
    if ($signature.PSIsContainer) { Fail "candidate updater signature is not a regular file" }
    $publicKey = (Resolve-Path -LiteralPath $PublicKeyPath -ErrorAction Stop).Path
    Validate-PublicKey $publicKey
    if (Test-Path -LiteralPath $OutputRoot) {
        Fail "candidate output directory already exists: $OutputRoot"
    }
    New-Item -ItemType Directory -Path $OutputRoot -Force | Out-Null
    Copy-Item -LiteralPath $installer.FullName -Destination (Join-Path $OutputRoot $installer.Name)
    Copy-Item -LiteralPath $signature.FullName -Destination (Join-Path $OutputRoot $signature.Name)
    Copy-Item -LiteralPath $publicKey -Destination (Join-Path $OutputRoot $publicKeyFileName)
    $installerPath = Join-Path $OutputRoot $installer.Name
    $signaturePath = Join-Path $OutputRoot $signature.Name
    $publicKeyOutputPath = Join-Path $OutputRoot $publicKeyFileName
    $metadata = [ordered]@{
        schema_version = 1
        source_sha = $sourceSha
        version = $version
        installer = $installer.Name
        installer_sha256 = Get-Sha256 $installerPath
        updater_signature = $signature.Name
        updater_signature_sha256 = Get-Sha256 $signaturePath
        updater_public_key = $publicKeyFileName
        updater_public_key_sha256 = Get-Sha256 $publicKeyOutputPath
    }
    [IO.File]::WriteAllText(
        (Join-Path $OutputRoot "candidate.json"),
        ($metadata | ConvertTo-Json -Compress),
        [Text.UTF8Encoding]::new($false))
    return Read-CandidateContract $OutputRoot $sourceSha $version
}

if ($Mode -eq "Create") {
    $contract = New-CandidateContract
} else {
    if ([string]::IsNullOrWhiteSpace($CandidateRoot)) {
        Fail "Validate mode requires CandidateRoot"
    }
    $contract = Read-CandidateContract $CandidateRoot $SourceSha (Get-ProjectVersion $RepositoryRoot)
}

Write-Output "CI candidate contract: PASS (source=$($contract.SourceSha); version=$($contract.Version); installer=$($contract.InstallerSha256); signature=$($contract.SignatureSha256); public_key=$($contract.PublicKeySha256))"
