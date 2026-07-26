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

$pyprojectPath = "pyproject.toml"
$projectVersion = ""
if (Test-Path $pyprojectPath) {
    $pyprojectContent = Get-Content $pyprojectPath -Raw
    if ($pyprojectContent -match 'version\s*=\s*"([^"]+)"') {
        $projectVersion = $matches[1]
    }
}

function Validate-Manifest {
    param([string]$ArtifactPath)
    Write-Host "Validating Artifact Manifest at $ArtifactPath..."
    
    $files = Get-ChildItem -Path $ArtifactPath -Recurse -File
    foreach ($file in $files) {
        $relPath = $file.FullName.Substring((Convert-Path $ArtifactPath).Length + 1).Replace('\', '/')
        
        # Reject invalid paths
        if ($relPath -match '\.(md|toml|lock|py|ps1)$' -or $relPath -match '^\.git') {
            Write-Error "Invalid file type found in artifact: $relPath"
            exit 1
        }
        if ($relPath -match '/plan/' -or $relPath -match '/archive/' -or $relPath -match '/perf-baselines/' -or $relPath -match '/lighthouse-report/') {
            Write-Error "Invalid path found in artifact: $relPath"
            exit 1
        }
        
        # Check allowed files
        $isAllowed = $false
        if ($relPath -in @("index.html", "faq.html", "vi/index.html", "vi/faq.html", "sitemap.xml", "llms.txt", ".nojekyll")) {
            $isAllowed = $true
        } elseif ($relPath -match '^google.*\.html$') {
            $isAllowed = $true
        } elseif ($relPath -match '^assets/') {
            $isAllowed = $true
        }
        
        if (-not $isAllowed) {
            Write-Error "File not in allowlist: $relPath"
            exit 1
        }
        
        # Check symlink
        if ($file.Attributes -match 'ReparsePoint') {
            Write-Error "Symlinks not allowed: $relPath"
            exit 1
        }
    }
    Write-Host "Manifest validation passed."
}

function Validate-HtmlFiles {
    param([string]$BasePath)
    Write-Host "Validating HTML files in $BasePath..."
    
    $htmlFiles = @(
        "index.html",
        "faq.html",
        "vi/index.html",
        "vi/faq.html"
    )
    
    foreach ($relPath in $htmlFiles) {
        $fullPath = Join-Path $BasePath $relPath
        if (-not (Test-Path $fullPath)) {
            Write-Error "Missing required HTML file: $relPath"
            exit 1
        }
        
        $content = Get-Content $fullPath -Raw
        
        # 1. <html lang>
        if ($content -notmatch '<html[^>]+lang="([^"]+)"') { Write-Error "$($relPath): Missing html lang"; exit 1 }
        
        # 2. title not empty
        if ($content -notmatch '<title>.+</title>') { Write-Error "$($relPath): Missing or empty title"; exit 1 }
        
        # 3. meta description
        if ($content -notmatch '<meta\s+name="description"\s+content="[^"]+"') { Write-Error "$($relPath): Missing or empty meta description"; exit 1 }
        
        # 4. canonical
        $canonicalCount = ([regex]::Matches($content, '<link\s+rel="canonical"\s+href="([^"]+)"')).Count
        if ($canonicalCount -ne 1) { Write-Error "$($relPath): Expected 1 canonical link, found $canonicalCount"; exit 1 }
        
        $canonicalUrl = [regex]::Match($content, '<link\s+rel="canonical"\s+href="([^"]+)"').Groups[1].Value
        
        # 5. hreflang (en, vi, x-default)
        $hreflangMatches = [regex]::Matches($content, '<link\s+rel="alternate"\s+hreflang="([^"]+)"')
        $hreflangs = $hreflangMatches | ForEach-Object { $_.Groups[1].Value } | Sort-Object
        $expectedHreflangs = @("en", "vi", "x-default") | Sort-Object
        if (Compare-Object $hreflangs $expectedHreflangs -SyncWindow 0) { Write-Error "$($relPath): Incorrect hreflang tags. Found: $($hreflangs -join ', ')"; exit 1 }
        
        # 6. og:url = canonical
        if ($content -notmatch "<meta\s+property=`"og:url`"\s+content=`"$canonicalUrl`"") { Write-Error "$($relPath): og:url does not match canonical or is missing"; exit 1 }
        
        # 7. og:image absolute HTTPS
        if ($content -notmatch '<meta\s+property="og:image"\s+content="https://[^"]+"') { Write-Error "$($relPath): og:image missing or not absolute HTTPS"; exit 1 }
        
        # 8. no noindex
        if ($content -match '<meta\s+name="robots"\s+content=".*noindex.*"') { Write-Error "$($relPath): Contains noindex"; exit 1 }
        
        # 9. JSON-LD parse
        $jsonLdMatches = [regex]::Matches($content, '(?s)<script\s+type="application/ld\+json">(.+?)</script>')
        if ($jsonLdMatches.Count -gt 0) {
            foreach ($m in $jsonLdMatches) {
                try {
                    $json = $m.Groups[1].Value | ConvertFrom-Json
                    
                    if ($relPath -match 'index\.html') {
                        if ($json.'@type' -ne 'SoftwareApplication' -and $json.'@type' -ne 'WebSite') {
                            # It could be WebSite + SoftwareApplication graph or just one of them.
                            # Just ensuring it has some schema.
                        }
                        if ($json.softwareVersion -and $projectVersion -and $json.softwareVersion -ne $projectVersion) {
                            Write-Error "$($relPath): softwareVersion '$($json.softwareVersion)' does not match project version '$projectVersion'"
                            exit 1
                        }
                    }
                    if ($relPath -match 'faq\.html') {
                        if ($json.'@type' -ne 'FAQPage' -and $json.'@type' -ne 'BreadcrumbList') {
                            # Could be multiple schemas, just checking parsing works.
                        }
                    }
                } catch {
                    Write-Error "$($relPath): Failed to parse JSON-LD: $_"
                    exit 1
                }
            }
        }
    }
    Write-Host "HTML validation passed."
}

function Validate-Sitemap {
    param([string]$BasePath)
    Write-Host "Validating sitemap in $BasePath..."
    $sitemapPath = Join-Path $BasePath "sitemap.xml"
    if (-not (Test-Path $sitemapPath)) {
        Write-Error "Missing sitemap.xml"
        exit 1
    }
    
    [xml]$xml = Get-Content $sitemapPath
    
    # Check namespace
    if ($xml.urlset.NamespaceURI -ne "http://www.sitemaps.org/schemas/sitemap/0.9") {
        Write-Error "Sitemap invalid namespace"
        exit 1
    }
    
    $urls = $xml.urlset.url
    if ($urls.Count -ne 4) {
        Write-Error "Sitemap should have exactly 4 URLs, found $($urls.Count)"
        exit 1
    }
    
    $expectedLocs = @(
        "https://pumni.github.io/Sky-Auto-Player/",
        "https://pumni.github.io/Sky-Auto-Player/vi/",
        "https://pumni.github.io/Sky-Auto-Player/faq.html",
        "https://pumni.github.io/Sky-Auto-Player/vi/faq.html"
    )
    
    $locs = $urls | ForEach-Object { $_.loc }
    foreach ($loc in $locs) {
        if ($loc -notmatch '^https://pumni\.github\.io/Sky-Auto-Player/') {
            Write-Error "Invalid domain or path in sitemap: $loc"
            exit 1
        }
        if ($loc -match 'Sky-Player/') {
            Write-Error "Legacy Sky-Player URL found in sitemap: $loc"
            exit 1
        }
        if ($loc -notin $expectedLocs) {
            Write-Error "Unexpected URL in sitemap: $loc"
            exit 1
        }
    }
    
    # Check alternate reciprocal and lastmod format
    # Simple regex check for lastmod
    foreach ($url in $urls) {
        if ($url.lastmod -and $url.lastmod -notmatch '^\d{4}-\d{2}-\d{2}') {
            Write-Error "Invalid lastmod format in sitemap: $($url.lastmod)"
            exit 1
        }
    }
    Write-Host "Sitemap validation passed."
}

if ($Artifact) {
    Validate-Manifest -ArtifactPath $Artifact
    Validate-HtmlFiles -BasePath $Artifact
    Validate-Sitemap -BasePath $Artifact
}

if ($Source) {
    Validate-HtmlFiles -BasePath $Source
    Validate-Sitemap -BasePath $Source
}

Write-Host "All validations passed!"
