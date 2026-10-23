[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'lib/startup_calibration_projection.ps1')

function Assert-ProjectionTest([bool]$Condition, [string]$Message) {
    if (-not $Condition) { throw "Startup calibration export self-test failed: $Message" }
}

$exporterTokens = $null
$exporterParseErrors = $null
[void][System.Management.Automation.Language.Parser]::ParseFile(
    (Join-Path $PSScriptRoot 'export_startup_calibration_evidence.ps1'),
    [ref]$exporterTokens,
    [ref]$exporterParseErrors
)
Assert-ProjectionTest ($exporterParseErrors.Count -eq 0) 'the production exporter has PowerShell parse errors'

function Test-ByteArrayEqual([byte[]]$Left, [byte[]]$Right) {
    if ($Left.Length -ne $Right.Length) { return $false }
    for ($index = 0; $index -lt $Left.Length; $index++) {
        if ($Left[$index] -ne $Right[$index]) { return $false }
    }
    return $true
}

function Get-ByteSha256([byte[]]$Bytes) {
    $sha = [System.Security.Cryptography.SHA256]::Create()
    try {
        [Convert]::ToHexString($sha.ComputeHash($Bytes)).ToLowerInvariant()
    }
    finally {
        $sha.Dispose()
    }
}

function Test-SidecarMatchesBytes([byte[]]$JsonBytes, [byte[]]$SidecarBytes, [string]$ExpectedName) {
    $sidecarText = [System.Text.Encoding]::UTF8.GetString($SidecarBytes)
    if ($sidecarText -cnotmatch '\A(?<hash>[0-9a-f]{64})  (?<name>[^\r\n]+)\n\z') { return $false }
    if ($Matches.name -cne $ExpectedName) { return $false }
    (Get-ByteSha256 -Bytes $JsonBytes) -ceq $Matches.hash
}

function Invoke-GitText([string]$Repository, [string[]]$Arguments) {
    $output = & git -C $Repository @Arguments 2>&1
    $exitCode = $LASTEXITCODE
    if ($exitCode -ne 0) {
        throw "git $($Arguments -join ' ') failed with exit code $($exitCode): $($output -join [string][char]10)"
    }
    ($output -join [string][char]10).TrimEnd()
}

function Get-GitBlobBytes([string]$Repository, [string]$ObjectId) {
    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = 'git'
    $startInfo.UseShellExecute = $false
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    [void]$startInfo.ArgumentList.Add('-C')
    [void]$startInfo.ArgumentList.Add($Repository)
    [void]$startInfo.ArgumentList.Add('cat-file')
    [void]$startInfo.ArgumentList.Add('blob')
    [void]$startInfo.ArgumentList.Add($ObjectId)

    $process = [System.Diagnostics.Process]::Start($startInfo)
    $buffer = [System.IO.MemoryStream]::new()
    $process.StandardOutput.BaseStream.CopyTo($buffer)
    $process.WaitForExit()
    $errorText = $process.StandardError.ReadToEnd()
    if ($process.ExitCode -ne 0) {
        throw "git cat-file blob failed with exit code $($process.ExitCode): $errorText"
    }
    $buffer.ToArray()
}

function Assert-CanonicalProjection([string]$JsonPath) {
    $jsonBytes = [System.IO.File]::ReadAllBytes($JsonPath)
    $fileName = [System.IO.Path]::GetFileName($JsonPath)
    $sidecarBytes = [System.IO.File]::ReadAllBytes("$JsonPath.sha256")
    Assert-ProjectionTest ($jsonBytes.Length -ge 4) "$fileName output was unexpectedly short"
    Assert-ProjectionTest (-not ($jsonBytes[0] -eq 239 -and $jsonBytes[1] -eq 187 -and $jsonBytes[2] -eq 191)) "$fileName output has a UTF-8 BOM"
    Assert-ProjectionTest (-not ($jsonBytes -contains [byte]13)) "$fileName output contains CR bytes"
    Assert-ProjectionTest ($jsonBytes[$jsonBytes.Length - 1] -eq 10) "$fileName output does not end in LF"
    Assert-ProjectionTest ($jsonBytes[$jsonBytes.Length - 2] -ne 10) "$fileName output has more than one trailing LF"
    Assert-ProjectionTest (-not ($sidecarBytes[0] -eq 239 -and $sidecarBytes[1] -eq 187 -and $sidecarBytes[2] -eq 191)) "$fileName sidecar has a UTF-8 BOM"
    Assert-ProjectionTest ($sidecarBytes[-1] -eq 10 -and $sidecarBytes[-2] -ne 10) "$fileName sidecar must end in exactly one LF"
    Assert-ProjectionTest (Test-SidecarMatchesBytes -JsonBytes $jsonBytes -SidecarBytes $sidecarBytes -ExpectedName $fileName) "$fileName sidecar does not match the exact JSON bytes"
    $jsonBytes
}

$temporaryRoot = [System.IO.Path]::GetFullPath(
    (Join-Path ([System.IO.Path]::GetTempPath()) ('sky-startup-calibration-export-' + [guid]::NewGuid().ToString('N')))
)
$temporaryParent = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath())
if (-not $temporaryRoot.StartsWith($temporaryParent, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw 'Temporary Git repository escaped the system temporary directory.'
}
$repository = Join-Path $temporaryRoot 'repository'
$utf8WithoutBom = [System.Text.UTF8Encoding]::new($false)
$lineFeed = [string][char]10
$carriageReturn = [string][char]13

try {
    [System.IO.Directory]::CreateDirectory($repository) | Out-Null
    Invoke-GitText -Repository $repository -Arguments @('init', '--quiet') | Out-Null
    Invoke-GitText -Repository $repository -Arguments @('config', 'core.autocrlf', 'true') | Out-Null

    $repositoryAttributes = Join-Path $repository '.gitattributes'
    Copy-Item -LiteralPath (Join-Path (Split-Path -Parent $PSScriptRoot) '.gitattributes') -Destination $repositoryAttributes
    Invoke-GitText -Repository $repository -Arguments @('add', '--', '.gitattributes') | Out-Null

    $lfFixture = @(
        '{'
        '  "nested": {'
        '    "label": "Māori 🎵",'
        '    "values": [1, 2, 3]'
        '  }'
        '}'
    ) -join $lineFeed
    $crlfFixture = $lfFixture.Replace($lineFeed, $carriageReturn + $lineFeed)
    $evidenceDirectory = Join-Path $repository 'docs/evidence'
    [System.IO.Directory]::CreateDirectory($evidenceDirectory) | Out-Null

    $cases = @(
        @{ Name = 'wake-calibration-qualification-20991231-lf.json'; Serialized = $lfFixture },
        @{ Name = 'wake-calibration-qualification-20991231-crlf.json'; Serialized = $crlfFixture }
    )
    $outputs = [System.Collections.Generic.List[object]]::new()
    foreach ($case in $cases) {
        $jsonPath = Join-Path $evidenceDirectory $case.Name
        $hash = Write-StartupCalibrationProjection -SerializedJson $case.Serialized -OutputPath $jsonPath
        $bytes = Assert-CanonicalProjection -JsonPath $jsonPath
        $text = [System.Text.Encoding]::UTF8.GetString($bytes)
        Assert-ProjectionTest ($text.Contains('Māori 🎵')) "$($case.Name) lost non-ASCII fixture data"
        Assert-ProjectionTest ($hash -ceq (Get-ByteSha256 -Bytes $bytes)) "$($case.Name) helper returned the wrong hash"
        $outputs.Add([pscustomobject]@{ Name = $case.Name; Path = $jsonPath; RelativePath = "docs/evidence/$($case.Name)"; Bytes = $bytes; Hash = $hash })
    }

    Assert-ProjectionTest (Test-ByteArrayEqual -Left $outputs[0].Bytes -Right $outputs[1].Bytes) 'LF and CRLF serializer inputs did not produce identical JSON bytes'
    Assert-ProjectionTest ($outputs[0].Hash -ceq $outputs[1].Hash) 'LF and CRLF serializer inputs produced different hashes'

    $good = $outputs[0]
    $goodSidecarBytes = [System.IO.File]::ReadAllBytes("$($good.Path).sha256")
    $wrongSidecarBytes = $utf8WithoutBom.GetBytes(('0' * 64) + "  $($good.Name)$lineFeed")
    Assert-ProjectionTest (-not (Test-SidecarMatchesBytes -JsonBytes $good.Bytes -SidecarBytes $wrongSidecarBytes -ExpectedName $good.Name)) 'A mismatched sidecar was accepted'
    $changedText = [System.Text.Encoding]::UTF8.GetString($good.Bytes).Replace('Māori', 'Maori')
    $changedBytes = $utf8WithoutBom.GetBytes($changedText)
    Assert-ProjectionTest (-not (Test-SidecarMatchesBytes -JsonBytes $changedBytes -SidecarBytes $goodSidecarBytes -ExpectedName $good.Name)) 'Changed JSON bytes were accepted by the sidecar check'

    foreach ($output in $outputs) {
        $relativeSidecarPath = "$($output.RelativePath).sha256"
        $attributeOutput = Invoke-GitText -Repository $repository -Arguments @('check-attr', 'eol', '--', $output.RelativePath, $relativeSidecarPath)
        Assert-ProjectionTest (($attributeOutput -split $lineFeed | Where-Object { $_ -match ': eol: lf$' }).Count -eq 2) "$($output.Name) JSON and sidecar do not both select eol=lf"
        Invoke-GitText -Repository $repository -Arguments @('add', '--', $output.RelativePath, $relativeSidecarPath) | Out-Null

        $jsonBlobId = Invoke-GitText -Repository $repository -Arguments @('rev-parse', ":$($output.RelativePath)")
        $sidecarBlobId = Invoke-GitText -Repository $repository -Arguments @('rev-parse', ":$relativeSidecarPath")
        $stagedJsonBytes = Get-GitBlobBytes -Repository $repository -ObjectId $jsonBlobId
        $stagedSidecarBytes = Get-GitBlobBytes -Repository $repository -ObjectId $sidecarBlobId
        Assert-ProjectionTest (Test-ByteArrayEqual -Left $output.Bytes -Right $stagedJsonBytes) "$($output.Name) bytes changed in the staged Git blob"
        Assert-ProjectionTest (Test-SidecarMatchesBytes -JsonBytes $stagedJsonBytes -SidecarBytes $stagedSidecarBytes -ExpectedName $output.Name) "$($output.Name) staged sidecar does not match staged JSON bytes"

        [System.IO.File]::Delete($output.Path)
        [System.IO.File]::Delete("$($output.Path).sha256")
        Invoke-GitText -Repository $repository -Arguments @('checkout-index', '--force', '--', $output.RelativePath, $relativeSidecarPath) | Out-Null
        $checkedOutJsonBytes = [System.IO.File]::ReadAllBytes($output.Path)
        $checkedOutSidecarBytes = [System.IO.File]::ReadAllBytes("$($output.Path).sha256")
        Assert-ProjectionTest (Test-ByteArrayEqual -Left $stagedJsonBytes -Right $checkedOutJsonBytes) "$($output.Name) changed during checkout with core.autocrlf=true"
        Assert-ProjectionTest (Test-ByteArrayEqual -Left $stagedSidecarBytes -Right $checkedOutSidecarBytes) "$($output.Name) sidecar changed during checkout with core.autocrlf=true"
        Assert-ProjectionTest (Test-SidecarMatchesBytes -JsonBytes $checkedOutJsonBytes -SidecarBytes $checkedOutSidecarBytes -ExpectedName $output.Name) "$($output.Name) checkout sidecar does not match checkout JSON bytes"
    }

    Write-Output 'Startup calibration exporter LF/hash synthetic tests: PASS'
}
finally {
    if (Test-Path -LiteralPath $temporaryRoot -PathType Container) {
        Remove-Item -LiteralPath $temporaryRoot -Recurse -Force
    }
}
