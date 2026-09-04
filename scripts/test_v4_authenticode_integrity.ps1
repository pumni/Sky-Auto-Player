param(
    [Parameter(Mandatory = $true)]
    [string]$Source
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'v4_authenticode_crypto.ps1')

function Find-SignTool {
    if (-not [string]::IsNullOrWhiteSpace($env:TAURI_WINDOWS_SIGNTOOL_PATH)) {
        $candidate = (Resolve-Path -LiteralPath $env:TAURI_WINDOWS_SIGNTOOL_PATH -ErrorAction Stop).Path
        if (Test-Path -LiteralPath $candidate -PathType Leaf) { return $candidate }
    }
    $command = Get-Command signtool.exe -ErrorAction SilentlyContinue
    if ($null -ne $command) { return $command.Source }
    $kits = Get-ItemProperty -Path 'HKLM:\SOFTWARE\Microsoft\Windows Kits\Installed Roots' -ErrorAction SilentlyContinue
    if ($null -ne $kits -and -not [string]::IsNullOrWhiteSpace($kits.KitsRoot10)) {
        $matches = @(Get-ChildItem -LiteralPath (Join-Path $kits.KitsRoot10 'bin') -Filter signtool.exe -File -Recurse -ErrorAction SilentlyContinue |
            Where-Object { $_.FullName -match '\\x64\\signtool\.exe$' } |
            Sort-Object FullName -Descending)
        if ($matches.Count -gt 0) { return $matches[0].FullName }
    }
    throw 'signtool.exe is not available for the Authenticode tamper regression'
}

$sourcePath = (Resolve-Path -LiteralPath $Source -ErrorAction Stop).Path
$sourceItem = Get-Item -LiteralPath $sourcePath -ErrorAction Stop
if ($sourceItem.PSIsContainer -or $sourceItem.Extension.ToLowerInvariant() -notin @('.exe', '.dll')) {
    throw "Authenticode tamper regression source must be a regular PE file: $Source"
}
$expectedThumbprint = ([string]$env:SKY_AUTHENTICODE_TEST_THUMBPRINT).Trim()
if ($expectedThumbprint -notmatch '^[0-9a-fA-F]{40}$') {
    throw 'Authenticode tamper regression requires the exact ephemeral test signer thumbprint'
}
$temporaryRoot = if ([string]::IsNullOrWhiteSpace($env:RUNNER_TEMP)) {
    [IO.Path]::GetTempPath()
} else {
    $env:RUNNER_TEMP
}
$temporaryRoot = [IO.Path]::GetFullPath($temporaryRoot)
$fixtureRoot = Join-Path $temporaryRoot ('sky-v4-authenticode-tamper-' + [guid]::NewGuid().ToString('N'))
$cleanPath = Join-Path $fixtureRoot 'clean.exe'
$tamperedPath = Join-Path $fixtureRoot 'tampered.exe'
$signTool = Find-SignTool

try {
    New-Item -ItemType Directory -Path $fixtureRoot -Force | Out-Null
    Copy-Item -LiteralPath $sourcePath -Destination $cleanPath -Force
    $existingSignature = Get-AuthenticodeSignature -LiteralPath $cleanPath
    if ([string]$existingSignature.Status -ne 'NotSigned') {
        & $signTool remove /s $cleanPath
        if ($LASTEXITCODE -ne 0) {
            throw "signtool.exe failed to remove the source signature for tamper regression (exit $LASTEXITCODE)"
        }
    }

    & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
        -File (Join-Path $PSScriptRoot 'sign_v4_authenticode.ps1') `
        -Path $cleanPath
    if ($LASTEXITCODE -ne 0) {
        throw "Ephemeral Authenticode signing failed for tamper regression (exit $LASTEXITCODE)"
    }

    & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
        -File (Join-Path $PSScriptRoot 'verify_v4_authenticode.ps1') `
        -Mode test `
        -Artifact $cleanPath
    if ($LASTEXITCODE -ne 0) {
        throw "Clean signed PE failed independent Authenticode verification (exit $LASTEXITCODE)"
    }
    Write-Host 'Authenticode tamper regression: clean signed PE PASS'

    Copy-Item -LiteralPath $cleanPath -Destination $tamperedPath -Force
    $tamperedBytes = [IO.File]::ReadAllBytes($tamperedPath)
    $layout = Get-AuthenticodePeLayout $tamperedBytes
    $firstSection = $layout.Sections | Sort-Object VirtualAddress, PointerToRawData | Select-Object -First 1
    $mutationOffset = [int]($firstSection.PointerToRawData + [Math]::Min(128, $firstSection.SizeOfRawData - 1))
    $tamperedBytes[$mutationOffset] = $tamperedBytes[$mutationOffset] -bxor 1
    [IO.File]::WriteAllBytes($tamperedPath, $tamperedBytes)

    & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
        -File (Join-Path $PSScriptRoot 'verify_v4_authenticode.ps1') `
        -Mode test `
        -Artifact $tamperedPath
    $tamperedExitCode = $LASTEXITCODE
    if ($tamperedExitCode -eq 0) {
        throw 'Tampered signed PE unexpectedly passed independent Authenticode verification'
    }
    Write-Host "Authenticode tamper regression: modified signed bytes rejected (exit=$tamperedExitCode)"
} finally {
    if (Test-Path -LiteralPath $fixtureRoot) {
        Remove-Item -LiteralPath $fixtureRoot -Recurse -Force -ErrorAction SilentlyContinue
    }
}

Write-Host 'Authenticode tamper regression: PASS'
