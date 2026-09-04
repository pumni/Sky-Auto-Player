Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$environmentNames = @(
    'SKY_AUTHENTICODE_MODE',
    'SKY_AUTHENTICODE_TEST_THUMBPRINT',
    'SKY_AUTHENTICODE_TEST_PFX_PATH',
    'SKY_AUTHENTICODE_TEST_PFX_PASSWORD'
)

function Clear-TestSigningEnvironment {
    foreach ($name in $environmentNames) {
        Remove-Item "Env:$name" -ErrorAction SilentlyContinue
    }
    $githubEnvironment = ([string]$env:GITHUB_ENV).Trim()
    if (-not [string]::IsNullOrWhiteSpace($githubEnvironment) -and
        (Test-Path -LiteralPath $githubEnvironment -PathType Leaf)) {
        $remaining = @(
            [IO.File]::ReadAllLines($githubEnvironment) |
                Where-Object {
                    $line = [string]$_
                    -not ($environmentNames | Where-Object { $line.StartsWith("$_=", [StringComparison]::Ordinal) })
                }
        )
        [IO.File]::WriteAllLines(
            $githubEnvironment,
            [string[]]$remaining,
            [Text.UTF8Encoding]::new($false))
    }
}

$thumbprint = ([string]$env:SKY_AUTHENTICODE_TEST_THUMBPRINT).Trim()
$pfxPath = ([string]$env:SKY_AUTHENTICODE_TEST_PFX_PATH).Trim()
if ([string]::IsNullOrWhiteSpace($pfxPath)) {
    Write-Host 'V4 Authenticode test certificate cleanup: no test PFX was published'
    Clear-TestSigningEnvironment
    exit 0
}
if ($thumbprint -notmatch '^[0-9a-fA-F]{40}$') {
    throw 'V4 Authenticode test certificate cleanup received an invalid SHA-1 thumbprint'
}

$temporaryRoot = if ([string]::IsNullOrWhiteSpace($env:RUNNER_TEMP)) {
    [IO.Path]::GetTempPath()
} else {
    $env:RUNNER_TEMP
}
$temporaryRoot = [IO.Path]::GetFullPath($temporaryRoot).TrimEnd('\', '/')
$resolvedPfxPath = [IO.Path]::GetFullPath($pfxPath)
$rootPrefix = $temporaryRoot + [IO.Path]::DirectorySeparatorChar
if (-not $resolvedPfxPath.StartsWith($rootPrefix, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'Refusing to remove a V4 Authenticode PFX outside RUNNER_TEMP'
}
if ([IO.Path]::GetFileName($resolvedPfxPath) -notmatch '^sky-v4-test-signing-[0-9a-fA-F]{32}\.pfx$') {
    throw 'Refusing to remove a V4 Authenticode PFX with an unexpected filename'
}
if (Test-Path -LiteralPath $resolvedPfxPath -PathType Leaf) {
    Remove-Item -LiteralPath $resolvedPfxPath -Force -ErrorAction Stop
}
if (Test-Path -LiteralPath $resolvedPfxPath) {
    throw 'Could not remove the ephemeral V4 Authenticode PFX'
}

Clear-TestSigningEnvironment
Write-Host "V4 Authenticode test certificate cleanup: PASS (thumbprint=$thumbprint, PFX removed)"
