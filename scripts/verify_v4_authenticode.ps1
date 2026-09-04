param(
    [Parameter(Mandatory = $true)]
    [ValidateSet("test", "production")]
    [string]$Mode,

    [Parameter(Mandatory = $true)]
    [string[]]$Artifact,

    [string]$Evidence
)

$ErrorActionPreference = "Stop"
if ($Artifact.Count -eq 0) { throw "At least one Authenticode artifact is required" }

$files = foreach ($path in $Artifact) {
    $resolved = (Resolve-Path -LiteralPath $path -ErrorAction Stop).Path
    $item = Get-Item -LiteralPath $resolved -ErrorAction Stop
    if (-not $item.PSIsContainer -and $item.Extension.ToLowerInvariant() -in @(".exe", ".dll")) {
        $item
    } else {
        throw "Authenticode evidence target must be a regular PE file: $path"
    }
}

$seen = @{}
$records = foreach ($file in $files) {
    if ($seen.ContainsKey($file.Name)) { throw "Duplicate Authenticode evidence filename: $($file.Name)" }
    $seen[$file.Name] = $true
    $signature = Get-AuthenticodeSignature -LiteralPath $file.FullName
    if ([string]$signature.Status -ne "Valid" -or $null -eq $signature.SignerCertificate) {
        throw "Authenticode signature is not valid for $($file.Name): $($signature.Status)"
    }
    [ordered]@{
        name = $file.Name
        status = [string]$signature.Status
        sha256 = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
        signer_thumbprint = ([string]$signature.SignerCertificate.Thumbprint).ToUpperInvariant()
        signer_subject = [string]$signature.SignerCertificate.Subject
    }
}

$payload = [ordered]@{
    schema_version = 1
    evidence_type = "authenticode-verification"
    mode = $Mode.ToLowerInvariant()
    files = @($records)
}

if ([string]::IsNullOrWhiteSpace($Evidence)) {
    Write-Host "V4 Authenticode verification: PASS ($($files.Count) PE file(s), mode=$Mode)"
} else {
    $evidenceParent = Split-Path -Parent $Evidence
    if (-not [string]::IsNullOrWhiteSpace($evidenceParent)) {
        New-Item -ItemType Directory -Path $evidenceParent -Force | Out-Null
    }
    $payload | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $Evidence -Encoding UTF8
    Write-Host "V4 Authenticode evidence written: $Evidence"
}
