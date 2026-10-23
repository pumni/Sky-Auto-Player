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

    $lineFeed = [string][char]10
    $canonicalJson = [System.Text.RegularExpressions.Regex]::Replace($SerializedJson, '\r\n?', $lineFeed)
    $canonicalJson = $canonicalJson.TrimEnd([char[]]@([char]13, [char]10)) + $lineFeed
    $utf8WithoutBom = [System.Text.UTF8Encoding]::new($false)
    [System.IO.File]::WriteAllText($OutputPath, $canonicalJson, $utf8WithoutBom)

    $outputHash = (Get-FileHash -LiteralPath $OutputPath -Algorithm SHA256).Hash.ToLowerInvariant()
    $sidecarText = "$outputHash  $([System.IO.Path]::GetFileName($OutputPath))$lineFeed"
    [System.IO.File]::WriteAllText("$OutputPath.sha256", $sidecarText, $utf8WithoutBom)

    $outputHash
}
