function Write-StartupCalibrationProjection {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [ValidateNotNullOrEmpty()]
        [string]$SerializedJson,
        [Parameter(Mandatory = $true)]
        [ValidateNotNullOrEmpty()]
        [string]$OutputPath
    )

    $scriptsRoot = Split-Path -Parent $PSScriptRoot
    $repositoryRoot = [System.IO.Path]::GetFullPath((Split-Path -Parent $scriptsRoot))
    $resolvedOutputPath = [System.IO.Path]::GetFullPath($OutputPath)
    $inputPath = Join-Path ([System.IO.Path]::GetTempPath()) (
        'sky-startup-calibration-projection-' + [guid]::NewGuid().ToString('N') + '.json'
    )
    $utf8WithoutBom = [System.Text.UTF8Encoding]::new($false)
    try {
        [System.IO.File]::WriteAllText($inputPath, $SerializedJson, $utf8WithoutBom)
        Push-Location -LiteralPath $repositoryRoot
        try {
            $commandOutput = @(
                & cargo --quiet xtask calibration project-evidence --input $inputPath --output $resolvedOutputPath 2>&1
            )
            $exitCode = $LASTEXITCODE
        }
        finally {
            Pop-Location
        }
        if ($exitCode -ne 0) {
            throw ("Rust startup calibration projection failed with exit code " + $exitCode + ": " + ($commandOutput -join [string][char]10))
        }
        $hashLines = @($commandOutput | Where-Object { ([string]$_).StartsWith('SHA256 ') })
        if ($hashLines.Count -ne 1) {
            throw 'Rust startup calibration projection omitted its SHA-256 result.'
        }
        $hashMatch = [regex]::Match([string]$hashLines[0], '^SHA256 ([0-9a-f]{64})$')
        if (-not $hashMatch.Success) {
            throw 'Rust startup calibration projection returned a malformed SHA-256 result.'
        }
        $hashMatch.Groups[1].Value
    }
    finally {
        if (Test-Path -LiteralPath $inputPath -PathType Leaf) {
            Remove-Item -LiteralPath $inputPath -Force
        }
    }
}
