function Get-V4ReleaseAuthorityAssetUploadUrl {
    param(
        [Parameter(Mandatory = $true)] [string]$UploadUrl,
        [Parameter(Mandatory = $true)] [string]$AssetName
    )

    if ($UploadUrl.Contains("?")) {
        throw "release asset upload base URL must not already contain a query"
    }

    $escapedAssetName = [Uri]::EscapeDataString($AssetName)
    $assetUrl = [string]::Concat($UploadUrl, "?name=", $escapedAssetName)
    if (-not $assetUrl.StartsWith(($UploadUrl + "?name="), [StringComparison]::Ordinal)) {
        throw "release asset upload URL construction failed"
    }
    return $assetUrl
}

$uploadUrlSelfTestBase = "https://uploads.github.com/repos/pumni/Sky-Auto-Player-Releases/releases/42/assets"
$uploadUrlSelfTestAsset = "fixture +#.bin"
$uploadUrlSelfTestExpected = "https://uploads.github.com/repos/pumni/Sky-Auto-Player-Releases/releases/42/assets?name=fixture%20%2B%23.bin"
$uploadUrlSelfTestActual = Get-V4ReleaseAuthorityAssetUploadUrl `
    -UploadUrl $uploadUrlSelfTestBase `
    -AssetName $uploadUrlSelfTestAsset
if ($uploadUrlSelfTestActual -ne $uploadUrlSelfTestExpected) {
    throw "release asset upload URL escaping self-test failed"
}
$uploadUrlSelfTestRejectedExistingQuery = $false
try {
    [void](Get-V4ReleaseAuthorityAssetUploadUrl `
        -UploadUrl ($uploadUrlSelfTestBase + "?existing=1") `
        -AssetName $uploadUrlSelfTestAsset)
} catch {
    $uploadUrlSelfTestRejectedExistingQuery = $true
}
if (-not $uploadUrlSelfTestRejectedExistingQuery) {
    throw "release asset upload URL query rejection self-test failed"
}
Remove-Variable uploadUrlSelfTestBase, uploadUrlSelfTestAsset, uploadUrlSelfTestExpected, uploadUrlSelfTestActual, uploadUrlSelfTestRejectedExistingQuery -ErrorAction SilentlyContinue

function Invoke-V4ReleaseAuthorityAssetUpload {
    param(
        [Parameter(Mandatory = $true)] [string]$UploadUrl,
        [Parameter(Mandatory = $true)] [string]$AssetName,
        [Parameter(Mandatory = $true)] [string]$FilePath,
        [Parameter(Mandatory = $true)] [string]$Token
    )

    if ($PSVersionTable.PSVersion -lt [Version]"7.4.0") {
        throw "release asset upload requires PowerShell 7.4 or newer"
    }
    if ([string]::IsNullOrWhiteSpace($Token)) { throw "release authority token is unavailable" }
    if (-not (Test-Path -LiteralPath $FilePath -PathType Leaf)) {
        throw "release asset upload file is missing"
    }

    $assetUrl = Get-V4ReleaseAuthorityAssetUploadUrl -UploadUrl $UploadUrl -AssetName $AssetName
    $client = [System.Net.Http.HttpClient]::new()
    $request = $null
    $fileStream = $null
    $content = $null
    $response = $null
    try {
        $fileStream = [System.IO.FileStream]::new(
            $FilePath,
            [System.IO.FileMode]::Open,
            [System.IO.FileAccess]::Read,
            [System.IO.FileShare]::Read
        )
        $content = [System.Net.Http.StreamContent]::new($fileStream)
        $content.Headers.ContentType = [System.Net.Http.Headers.MediaTypeHeaderValue]::new(
            "application/octet-stream"
        )
        $fileLength = [int64]$fileStream.Length
        $content.Headers.ContentLength = $fileLength
        if ($content.Headers.ContentLength -ne $fileLength) {
            throw "release asset upload content length is not exact"
        }
        $request = [System.Net.Http.HttpRequestMessage]::new(
            [System.Net.Http.HttpMethod]::Post,
            [Uri]$assetUrl
        )
        $request.Headers.UserAgent.ParseAdd("Sky-Auto-Player-v4-release-pipeline/1.0")
        [void]$request.Headers.Accept.Add(
            [System.Net.Http.Headers.MediaTypeWithQualityHeaderValue]::new(
                "application/vnd.github+json"
            )
        )
        [void]$request.Headers.Add("X-GitHub-Api-Version", "2026-03-10")
        $request.Headers.Authorization = [System.Net.Http.Headers.AuthenticationHeaderValue]::new(
            "Bearer",
            $Token
        )
        $request.Content = $content
        $response = $client.SendAsync($request).GetAwaiter().GetResult()
        $responseBody = $response.Content.ReadAsStringAsync().GetAwaiter().GetResult()
        if ($response.StatusCode -ne [System.Net.HttpStatusCode]::Created) {
            throw "release authority asset upload failed"
        }
        return ($responseBody | ConvertFrom-Json)
    } finally {
        if ($null -ne $response) { $response.Dispose() }
        if ($null -ne $request) { $request.Dispose() }
        if ($null -ne $content) { $content.Dispose() }
        if ($null -ne $fileStream) { $fileStream.Dispose() }
        $client.Dispose()
    }
}
