function Select-V4ReleaseByTag {
    param(
        [AllowNull()]
        [object]$DirectRelease,

        [AllowNull()]
        [object[]]$ReleaseCollection,

        [Parameter(Mandatory = $true)]
        [string]$Tag
    )

    if ($null -ne $DirectRelease) {
        if ([string]$DirectRelease.tag_name -ne $Tag) {
            throw "V4 release lookup failed closed: direct release tag does not match the requested tag"
        }
        return $DirectRelease
    }

    $matches = @($ReleaseCollection | Where-Object { [string]$_.tag_name -eq $Tag })
    if ($matches.Count -gt 1) {
        throw "V4 release lookup failed closed: duplicate releases use the requested tag"
    }
    if ($matches.Count -eq 1) {
        return $matches[0]
    }
    return $null
}
