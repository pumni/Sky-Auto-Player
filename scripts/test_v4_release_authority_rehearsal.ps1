[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [switch]$ConfirmDisposable,

    [string]$AuthorityTokenEnv = "V4_RELEASE_AUTHORITY_TOKEN"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if (-not $ConfirmDisposable) {
    throw "Authority rehearsal requires -ConfirmDisposable and creates only a uniquely tagged draft that is deleted before exit"
}

$authorityRepository = "pumni/Sky-Auto-Player-Releases"
$token = [Environment]::GetEnvironmentVariable($AuthorityTokenEnv, "Process")
if ([string]::IsNullOrWhiteSpace($token)) {
    throw "Authority rehearsal requires the bounded authority token environment variable"
}

$rehearsalRoot = Join-Path ([IO.Path]::GetTempPath()) ("sky-v4-authority-rehearsal-" + [guid]::NewGuid().ToString("N"))
$artifactName = "v4-authority-upload-rehearsal-" + [guid]::NewGuid().ToString("N") + ".txt"
$artifactPath = Join-Path $rehearsalRoot $artifactName
$downloadPath = Join-Path $rehearsalRoot "downloaded.txt"
$errorPath = Join-Path $rehearsalRoot "gh-error.log"
$tag = "v4-authority-rehearsal-" + [guid]::NewGuid().ToString("N")
$releaseId = $null
$oldGhToken = [Environment]::GetEnvironmentVariable("GH_TOKEN", "Process")
$failure = $null

function Invoke-GhBinaryOutput {
    param(
        [Parameter(Mandatory = $true)] [string[]]$Arguments,
        [Parameter(Mandatory = $true)] [string]$OutputPath,
        [Parameter(Mandatory = $true)] [string]$ErrorPath
    )
    if ($PSVersionTable.PSVersion -lt [Version]"7.4.0") {
        throw "binary release-asset download requires PowerShell 7.4 or newer"
    }
    if ([string]::IsNullOrWhiteSpace($OutputPath)) {
        throw "binary release-asset output path is required"
    }

    $ghPath = (Get-Command gh -CommandType Application -ErrorAction Stop | Select-Object -First 1).Source
    $startInfo = [Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $ghPath
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    foreach ($argument in $Arguments) {
        [void]$startInfo.ArgumentList.Add([string]$argument)
    }

    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    $outputStream = $null
    $stderrTask = $null
    $started = $false
    try {
        $outputStream = [IO.File]::Open(
            $OutputPath,
            [IO.FileMode]::Create,
            [IO.FileAccess]::Write,
            [IO.FileShare]::None
        )
        $started = $process.Start()
        if (-not $started) { throw "could not start GitHub CLI for binary release-asset download" }
        $stderrTask = $process.StandardError.ReadToEndAsync()
        $process.StandardOutput.BaseStream.CopyTo($outputStream)
        $process.WaitForExit()
        $stderr = $stderrTask.GetAwaiter().GetResult()
        [IO.File]::WriteAllText($ErrorPath, $stderr, [Text.UTF8Encoding]::new($false))
        return [int]$process.ExitCode
    } finally {
        if ($started -and -not $process.HasExited) {
            $process.Kill()
            $process.WaitForExit()
        }
        if ($null -ne $outputStream) { $outputStream.Dispose() }
        $process.Dispose()
    }
}

function Invoke-AuthorityApi {
    param(
        [Parameter(Mandatory = $true)] [string[]]$Arguments,
        [switch]$BinaryOutput,
        [switch]$AllowNotFound,
        [switch]$NoJson,
        [string]$OutputPath
    )
    try {
        [Environment]::SetEnvironmentVariable("GH_TOKEN", $token, "Process")
        if ($BinaryOutput) {
            $exitCode = Invoke-GhBinaryOutput `
                -Arguments $Arguments -OutputPath $OutputPath -ErrorPath $errorPath
        } else {
            $result = & gh @Arguments 2>$errorPath
            $exitCode = $LASTEXITCODE
        }
    } finally {
        if ($null -eq $oldGhToken) {
            [Environment]::SetEnvironmentVariable("GH_TOKEN", $null, "Process")
        } else {
            [Environment]::SetEnvironmentVariable("GH_TOKEN", $oldGhToken, "Process")
        }
    }
    if ($exitCode -ne 0) {
        $errorText = if (Test-Path -LiteralPath $errorPath) { Get-Content -LiteralPath $errorPath -Raw } else { "" }
        if ($AllowNotFound -and $errorText -match '(?i)(404|not found)') { return $null }
        throw "release authority rehearsal API request failed"
    }
    if ($BinaryOutput) { return $null }
    if ($NoJson) { return ($result -join "`n") }
    return (($result -join "`n") | ConvertFrom-Json)
}

try {
    New-Item -ItemType Directory -Path $rehearsalRoot -Force | Out-Null
    [IO.File]::WriteAllText(
        $artifactPath,
        "throwaway v4 authority upload rehearsal $([guid]::NewGuid().ToString('N'))`n",
        [Text.UTF8Encoding]::new($false)
    )
    $expectedHash = (Get-FileHash -LiteralPath $artifactPath -Algorithm SHA256).Hash.ToLowerInvariant()
    $expectedSize = (Get-Item -LiteralPath $artifactPath).Length

    $policy = Invoke-AuthorityApi -Arguments @(
        "api", "repos/$authorityRepository/immutable-releases"
    )
    if (-not [bool]$policy.enabled) {
        throw "release authority immutable-releases policy is not enabled"
    }

    $releasePayload = Join-Path $rehearsalRoot "release.json"
    [ordered]@{
        tag_name = $tag
        target_commitish = "main"
        name = "V4 authority upload rehearsal $tag"
        body = "Disposable API rehearsal only; this draft is deleted before the script exits."
        draft = $true
        prerelease = $true
    } | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $releasePayload -Encoding utf8
    $release = Invoke-AuthorityApi -Arguments @(
        "api", "--method", "POST", "repos/$authorityRepository/releases", "--input", $releasePayload
    )
            $releaseId = [int64]$release.id
    if (-not $release.draft -or [string]$release.tag_name -ne $tag) {
        throw "authority rehearsal did not create the expected draft"
    }
    $uploadUrl = ([string]$release.upload_url) -replace '\{\?name,label\}$', ''
    if ($uploadUrl -notmatch '^https://uploads\.github\.com/repos/[^/]+/[^/]+/releases/\d+/assets$') {
        throw "authority rehearsal did not receive the release-specific upload_url"
    }
    $uploaded = Invoke-AuthorityApi -Arguments @(
        "api", "--method", "POST", "$uploadUrl?name=$([Uri]::EscapeDataString($artifactName))",
        "--header", "Content-Type: text/plain", "--input", $artifactPath
    )
    if ([string]$uploaded.name -ne $artifactName -or [int64]$uploaded.size -ne [int64]$expectedSize) {
        throw "authority rehearsal upload response did not preserve the exact asset identity"
    }

    $rehearsed = Invoke-AuthorityApi -Arguments @(
        "api", "repos/$authorityRepository/releases/$releaseId"
    )
    $asset = @($rehearsed.assets | Where-Object { [string]$_.name -eq $artifactName })
    if ($asset.Count -ne 1) { throw "authority rehearsal draft asset was not found" }
    Invoke-AuthorityApi -Arguments @(
        "api", [string]$asset[0].url, "--header", "Accept: application/octet-stream"
    ) -BinaryOutput -OutputPath $downloadPath
    if ((Get-Item -LiteralPath $downloadPath).Length -ne $expectedSize -or
        (Get-FileHash -LiteralPath $downloadPath -Algorithm SHA256).Hash.ToLowerInvariant() -ne $expectedHash) {
        throw "authority rehearsal downloaded bytes did not match the throwaway upload"
    }
}
catch {
    $failure = $_
}
finally {
    try {
        if ($null -ne $releaseId) {
            Invoke-AuthorityApi -Arguments @(
                "api", "--method", "DELETE", "repos/$authorityRepository/releases/$releaseId"
            ) -NoJson | Out-Null
        }
        Invoke-AuthorityApi -Arguments @(
            "api", "--method", "DELETE", "repos/$authorityRepository/git/refs/tags/$tag"
        ) -AllowNotFound -NoJson | Out-Null
    }
    catch {
        if ($null -eq $failure) { $failure = $_ }
    }
    if ($null -eq $oldGhToken) {
        [Environment]::SetEnvironmentVariable("GH_TOKEN", $null, "Process")
    } else {
        [Environment]::SetEnvironmentVariable("GH_TOKEN", $oldGhToken, "Process")
    }
    if (Test-Path -LiteralPath $rehearsalRoot) {
        Remove-Item -LiteralPath $rehearsalRoot -Recurse -Force -ErrorAction SilentlyContinue
    }
}

if ($null -ne $failure) {
    throw "V4 release authority rehearsal failed closed; disposable cleanup was attempted"
}
Write-Host "V4 release authority upload rehearsal: PASS (draft upload/download exact bytes; draft and tag deleted)"
