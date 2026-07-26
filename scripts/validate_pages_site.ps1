param (
    [Parameter()]
    [string]$Source,

    [Parameter()]
    [string]$Artifact
)

$ErrorActionPreference = 'Stop'

if (-not $Source -and -not $Artifact) {
    Write-Error "Must provide at least -Source or -Artifact"
    exit 1
}

function Fail {
    param([string]$Message)
    Write-Error $Message
    exit 1
}

function Validate-Manifest {
    param([string]$ArtifactPath)
    Write-Host "Validating artifact allowlist at $ArtifactPath..."

    $root = Convert-Path $ArtifactPath
    $files = Get-ChildItem -LiteralPath $ArtifactPath -Recurse -File
    foreach ($file in $files) {
        $relPath = $file.FullName.Substring($root.Length + 1).Replace('\', '/')

        if ($relPath -match '\.(md|toml|lock|py|ps1)$' -or $relPath -match '^\.git') {
            Fail "Invalid file type found in artifact: $relPath"
        }
        if ($relPath -match '(^|/)(plan|archive|perf-baselines|lighthouse-report)(/|$)') {
            Fail "Invalid path found in artifact: $relPath"
        }

        $rootFiles = @(
            'index.html', 'faq.html', 'faq/index.html', 'vi/index.html', 'vi/faq.html', 'vi/faq/index.html',
            'robots.txt', 'llms.txt', 'favicon.ico', 'favicon.svg',
            'sitemap-index.xml', 'sitemap-0.xml'
        )
        $isAllowed = $relPath -in $rootFiles -or
            $relPath -match '^google[^/]*\.html$' -or
            $relPath -match '^assets/' -or
            $relPath -match '^_astro/'

        if (-not $isAllowed) {
            Fail "File not in allowlist: $relPath"
        }

        if ($file.Attributes -match 'ReparsePoint') {
            Fail "Symlinks not allowed: $relPath"
        }
    }
    Write-Host "Manifest validation passed."
}

function Validate-CanonicalHtml {
    param([string]$BasePath)
    Write-Host "Validating canonical HTML files in $BasePath..."

    $canonicalPages = @{
        'index.html' = 'https://pumni.github.io/Sky-Auto-Player/'
        'faq/index.html' = 'https://pumni.github.io/Sky-Auto-Player/faq/'
        'vi/index.html' = 'https://pumni.github.io/Sky-Auto-Player/vi/'
        'vi/faq/index.html' = 'https://pumni.github.io/Sky-Auto-Player/vi/faq/'
    }

    foreach ($entry in $canonicalPages.GetEnumerator()) {
        $relPath = $entry.Key
        $fullPath = Join-Path $BasePath $relPath
        if (-not (Test-Path -LiteralPath $fullPath)) {
            Fail "Missing required HTML file: $relPath"
        }

        $content = Get-Content -LiteralPath $fullPath -Raw
        if ($content -notmatch '<html[^>]+lang="(en|vi)"') { Fail "$($relPath): Missing html lang" }
        if ($content -notmatch '<title>.+</title>') { Fail "$($relPath): Missing or empty title" }
        if ($content -notmatch '<meta\s+name="description"\s+content="[^"]+"') { Fail "$($relPath): Missing or empty meta description" }

        $canonicalMatch = [regex]::Match($content, '<link\s+rel="canonical"\s+href="([^"]+)"')
        if (-not $canonicalMatch.Success) { Fail "$($relPath): Missing canonical link" }
        if ($canonicalMatch.Groups[1].Value -ne $entry.Value) { Fail "$($relPath): Canonical does not match expected route" }

        $hreflangMatches = [regex]::Matches($content, '<link\s+rel="alternate"\s+hreflang="([^"]+)"')
        $hreflangs = @($hreflangMatches | ForEach-Object { $_.Groups[1].Value } | Sort-Object)
        $expectedHreflangs = @('en', 'vi', 'x-default') | Sort-Object
        if (Compare-Object $hreflangs $expectedHreflangs -SyncWindow 0) { Fail "$($relPath): Incorrect hreflang tags" }

        if ($content -notmatch "<meta\s+property=`"og:url`"\s+content=`"$([regex]::Escape($entry.Value))`"") {
            Fail "$($relPath): og:url does not match canonical"
        }
        if ($content -notmatch '<meta\s+property="og:image"\s+content="https://[^/"]+') { Fail "$($relPath): og:image missing or not absolute HTTPS" }
        if ($content -match '<meta\s+name="robots"\s+content=".*noindex.*"') { Fail "$($relPath): Contains noindex" }

        $jsonLdMatches = [regex]::Matches($content, '(?s)<script\s+type="application/ld\+json">(.+?)</script>')
        if ($jsonLdMatches.Count -ne 1) { Fail "$($relPath): Expected exactly one JSON-LD block" }
        try {
            $json = $jsonLdMatches[0].Groups[1].Value | ConvertFrom-Json
        } catch {
            Fail "$($relPath): Failed to parse JSON-LD: $_"
        }
        if ($relPath -match 'faq/index\.html' -and $json.'@type' -ne 'FAQPage') {
            Fail "$($relPath): Expected FAQPage JSON-LD"
        }
        if ($relPath -notmatch 'faq/index\.html' -and $json.'@type' -ne 'SoftwareApplication') {
            Fail "$($relPath): Expected SoftwareApplication JSON-LD"
        }
    }
    Write-Host "Canonical HTML validation passed."
}

function Validate-LegacyRedirects {
    param([string]$BasePath)
    $redirects = @{
        'faq.html' = './faq/'
        'vi/faq.html' = './faq/'
    }
    foreach ($entry in $redirects.GetEnumerator()) {
        $fullPath = Join-Path $BasePath $entry.Key
        if (-not (Test-Path -LiteralPath $fullPath)) { Fail "Missing legacy redirect: $($entry.Key)" }
        $content = Get-Content -LiteralPath $fullPath -Raw
        if ($content -notmatch 'http-equiv="refresh"' -or $content -notmatch [regex]::Escape("url=$($entry.Value)")) {
            Fail "Invalid legacy redirect: $($entry.Key)"
        }
    }
}

function Validate-GoogleVerification {
    param([string]$BasePath)
    $matches = Get-ChildItem -LiteralPath $BasePath -Filter 'google*.html' -File
    if ($matches.Count -lt 1) { Fail 'Missing Google verification file' }
    foreach ($file in $matches) {
        $content = Get-Content -LiteralPath $file.FullName -Raw
        if ($content -notmatch 'google-site-verification:') { Fail "Invalid Google verification file: $($file.Name)" }
    }
}

function Validate-Sitemap {
    param([string]$BasePath)
    Write-Host "Validating sitemap index in $BasePath..."

    $indexPath = Join-Path $BasePath 'sitemap-index.xml'
    if (-not (Test-Path -LiteralPath $indexPath)) { Fail 'Missing sitemap-index.xml' }
    [xml]$indexXml = Get-Content -LiteralPath $indexPath -Raw
    if ($indexXml.sitemapindex.NamespaceURI -ne 'http://www.sitemaps.org/schemas/sitemap/0.9') {
        Fail 'Sitemap index invalid namespace'
    }

    $childLoc = @($indexXml.sitemapindex.sitemap.loc)[0]
    if (-not $childLoc) { Fail 'Sitemap index has no child sitemap' }
    $childName = Split-Path ([Uri]$childLoc) -Leaf
    $sitemapPath = Join-Path $BasePath $childName
    if (-not (Test-Path -LiteralPath $sitemapPath)) { Fail "Missing child sitemap: $childName" }

    [xml]$xml = Get-Content -LiteralPath $sitemapPath -Raw
    if ($xml.urlset.NamespaceURI -ne 'http://www.sitemaps.org/schemas/sitemap/0.9') {
        Fail 'Sitemap invalid namespace'
    }

    $urls = @($xml.urlset.url)
    if ($urls.Count -ne 4) { Fail "Sitemap should have exactly 4 URLs, found $($urls.Count)" }
    $expectedLocs = @(
        'https://pumni.github.io/Sky-Auto-Player/',
        'https://pumni.github.io/Sky-Auto-Player/faq/',
        'https://pumni.github.io/Sky-Auto-Player/vi/',
        'https://pumni.github.io/Sky-Auto-Player/vi/faq/'
    )
    foreach ($url in $urls) {
        if ($url.loc -notin $expectedLocs) { Fail "Unexpected URL in sitemap: $($url.loc)" }
        if ($url.loc -notmatch '^https://pumni\.github\.io/Sky-Auto-Player/') { Fail "Invalid sitemap domain: $($url.loc)" }
    }
    Write-Host 'Sitemap validation passed.'
}

if ($Artifact) {
    Validate-Manifest -ArtifactPath $Artifact
    Validate-CanonicalHtml -BasePath $Artifact
    Validate-LegacyRedirects -BasePath $Artifact
    Validate-GoogleVerification -BasePath $Artifact
    Validate-Sitemap -BasePath $Artifact
}

if ($Source) {
    Validate-CanonicalHtml -BasePath $Source
    Validate-LegacyRedirects -BasePath $Source
    Validate-GoogleVerification -BasePath $Source
    Validate-Sitemap -BasePath $Source
}

Write-Host 'All validations passed!'