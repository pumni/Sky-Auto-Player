$ErrorActionPreference = 'Stop'

try {
    Add-Type -AssemblyName System.Security.Cryptography.Pkcs -ErrorAction Stop
    Add-Type -AssemblyName System.Formats.Asn1 -ErrorAction Stop
} catch {
    throw 'The System.Security.Cryptography.Pkcs assembly is required for independent Authenticode verification'
}

function Read-UInt16LittleEndian {
    param(
        [Parameter(Mandatory = $true)] [byte[]]$Bytes,
        [Parameter(Mandatory = $true)] [int]$Offset
    )
    if ($Offset -lt 0 -or $Offset + 2 -gt $Bytes.Length) {
        throw 'PE header contains an out-of-range 16-bit field'
    }
    return [BitConverter]::ToUInt16($Bytes, $Offset)
}

function Read-UInt32LittleEndian {
    param(
        [Parameter(Mandatory = $true)] [byte[]]$Bytes,
        [Parameter(Mandatory = $true)] [int]$Offset
    )
    if ($Offset -lt 0 -or $Offset + 4 -gt $Bytes.Length) {
        throw 'PE header contains an out-of-range 32-bit field'
    }
    return [BitConverter]::ToUInt32($Bytes, $Offset)
}

function Get-AuthenticodePeLayout {
    param(
        [Parameter(Mandatory = $true)] [byte[]]$Bytes
    )
    if ($Bytes.Length -lt 64 -or (Read-UInt16LittleEndian $Bytes 0) -ne 0x5a4d) {
        throw 'Authenticode integrity target is not a PE image'
    }
    $peOffset = [int](Read-UInt32LittleEndian $Bytes 0x3c)
    if ($peOffset -lt 0 -or $peOffset + 24 -gt $Bytes.Length) {
        throw 'PE image has an invalid PE header offset'
    }
    if (-not ([Text.Encoding]::ASCII.GetString($Bytes, $peOffset, 4) -eq "PE`0`0")) {
        throw 'PE image signature is invalid'
    }

    $numberOfSections = [int](Read-UInt16LittleEndian $Bytes ($peOffset + 6))
    $optionalHeaderSize = [int](Read-UInt16LittleEndian $Bytes ($peOffset + 20))
    $optionalHeaderOffset = $peOffset + 24
    $optionalHeaderEnd = $optionalHeaderOffset + $optionalHeaderSize
    if ($numberOfSections -le 0 -or $optionalHeaderSize -le 0 -or $optionalHeaderEnd -gt $Bytes.Length) {
        throw 'PE image has invalid section or optional-header bounds'
    }

    $optionalMagic = Read-UInt16LittleEndian $Bytes $optionalHeaderOffset
    $dataDirectoryOffset = switch ($optionalMagic) {
        0x10b { $optionalHeaderOffset + 96; break }
        0x20b { $optionalHeaderOffset + 112; break }
        default { throw "PE image has unsupported optional-header magic: 0x$('{0:X}' -f $optionalMagic)" }
    }
    $numberOfDirectoriesOffset = if ($optionalMagic -eq 0x10b) {
        $optionalHeaderOffset + 92
    } else {
        $optionalHeaderOffset + 108
    }
    if ($numberOfDirectoriesOffset + 4 -gt $optionalHeaderEnd -or
        (Read-UInt32LittleEndian $Bytes $numberOfDirectoriesOffset) -lt 5) {
        throw 'PE image has no Certificate Table data directory'
    }
    $checksumOffset = $optionalHeaderOffset + 64
    $certificateDirectoryOffset = $dataDirectoryOffset + (8 * 4)
    $sizeOfHeaders = [long](Read-UInt32LittleEndian $Bytes ($optionalHeaderOffset + 60))
    if ($checksumOffset + 4 -gt $optionalHeaderEnd -or
        $certificateDirectoryOffset + 8 -gt $optionalHeaderEnd -or
        $sizeOfHeaders -le 0 -or $sizeOfHeaders -gt $Bytes.Length) {
        throw 'PE image optional-header fields are truncated'
    }

    $certificateTableOffset = [long](Read-UInt32LittleEndian $Bytes $certificateDirectoryOffset)
    $certificateTableSize = [long](Read-UInt32LittleEndian $Bytes ($certificateDirectoryOffset + 4))
    if ($certificateTableOffset -le 0 -or $certificateTableSize -lt 8 -or
        $certificateTableOffset + $certificateTableSize -gt $Bytes.Length) {
        throw 'PE image has an invalid or missing Authenticode certificate table'
    }

    $sectionTableOffset = $optionalHeaderEnd
    $sectionHeaderSize = 40
    if ($sectionTableOffset + ($numberOfSections * $sectionHeaderSize) -gt $Bytes.Length) {
        throw 'PE image section table is truncated'
    }
    if ($sizeOfHeaders -lt $sectionTableOffset + ($numberOfSections * $sectionHeaderSize)) {
        throw 'PE image SizeOfHeaders does not cover the complete section table'
    }
    $sections = foreach ($index in 0..($numberOfSections - 1)) {
        $sectionOffset = $sectionTableOffset + ($index * $sectionHeaderSize)
        $virtualAddress = [long](Read-UInt32LittleEndian $Bytes ($sectionOffset + 12))
        $sizeOfRawData = [long](Read-UInt32LittleEndian $Bytes ($sectionOffset + 16))
        $pointerToRawData = [long](Read-UInt32LittleEndian $Bytes ($sectionOffset + 20))
        if ($sizeOfRawData -eq 0) { continue }
        if ($pointerToRawData -le 0 -or $pointerToRawData + $sizeOfRawData -gt $Bytes.Length) {
            throw 'PE image section raw-data bounds are invalid'
        }
        [pscustomobject]@{
            VirtualAddress = $virtualAddress
            PointerToRawData = $pointerToRawData
            SizeOfRawData = $sizeOfRawData
        }
    }
    if (@($sections).Count -eq 0) {
        throw 'PE image has no section data to hash'
    }

    [pscustomobject]@{
        ChecksumOffset = [long]$checksumOffset
        CertificateDirectoryOffset = [long]$certificateDirectoryOffset
        CertificateTableOffset = $certificateTableOffset
        CertificateTableSize = $certificateTableSize
        SizeOfHeaders = $sizeOfHeaders
        Sections = @($sections)
    }
}

function Add-AuthenticodeHashRange {
    param(
        [Parameter(Mandatory = $true)] [System.Security.Cryptography.IncrementalHash]$Hasher,
        [Parameter(Mandatory = $true)] [byte[]]$Bytes,
        [Parameter(Mandatory = $true)] [long]$Start,
        [Parameter(Mandatory = $true)] [long]$End,
        [Parameter(Mandatory = $true)] [object[]]$Exclusions
    )
    if ($Start -ge $End) { return }
    $cursor = $Start
    foreach ($exclusion in ($Exclusions | Sort-Object Start)) {
        if ($exclusion.End -le $cursor) { continue }
        if ($exclusion.Start -ge $End) { break }
        $cutStart = [math]::Max($cursor, [long]$exclusion.Start)
        if ($cutStart -gt $cursor) {
            $Hasher.AppendData($Bytes, [int]$cursor, [int]($cutStart - $cursor))
        }
        $cursor = [math]::Max($cursor, [math]::Min($End, [long]$exclusion.End))
        if ($cursor -ge $End) { return }
    }
    if ($cursor -lt $End) {
        $Hasher.AppendData($Bytes, [int]$cursor, [int]($End - $cursor))
    }
}

function Get-AuthenticodeImageDigest {
    param(
        [Parameter(Mandatory = $true)] [byte[]]$Bytes,
        [Parameter(Mandatory = $true)] $Layout,
        [Parameter(Mandatory = $true)] [System.Security.Cryptography.HashAlgorithmName]$HashAlgorithm
    )
    $exclusions = @(
        [pscustomobject]@{ Start = $Layout.ChecksumOffset; End = $Layout.ChecksumOffset + 4 },
        [pscustomobject]@{ Start = $Layout.CertificateDirectoryOffset; End = $Layout.CertificateDirectoryOffset + 8 },
        [pscustomobject]@{ Start = $Layout.CertificateTableOffset; End = $Layout.CertificateTableOffset + $Layout.CertificateTableSize }
    )
    $hasher = [System.Security.Cryptography.IncrementalHash]::CreateHash($HashAlgorithm)
    try {
        Add-AuthenticodeHashRange $hasher $Bytes 0 $Layout.SizeOfHeaders $exclusions
        $sumOfBytesHashed = [long]$Layout.SizeOfHeaders
        foreach ($section in ($Layout.Sections | Sort-Object PointerToRawData, VirtualAddress)) {
            Add-AuthenticodeHashRange $hasher $Bytes $section.PointerToRawData ($section.PointerToRawData + $section.SizeOfRawData) $exclusions
            $sumOfBytesHashed += $section.SizeOfRawData
        }
        $extraDataEnd = [long]$Bytes.Length - $Layout.CertificateTableSize
        if ($extraDataEnd -lt $sumOfBytesHashed) {
            throw 'PE image certificate table overlaps the Authenticode hashed image'
        }
        if ($extraDataEnd -gt $sumOfBytesHashed) {
            Add-AuthenticodeHashRange $hasher $Bytes $sumOfBytesHashed $extraDataEnd $exclusions
        }
        $paddingLength = (8 - ($Bytes.Length % 8)) % 8
        if ($paddingLength -gt 0) {
            $Hasher.AppendData([byte[]]::new($paddingLength))
        }
        return $hasher.GetHashAndReset()
    } finally {
        $hasher.Dispose()
    }
}

function Get-AuthenticodeSpcDigest {
    param(
        [Parameter(Mandatory = $true)] [byte[]]$Content
    )
    $reader = [System.Formats.Asn1.AsnReader]::new(
        $Content,
        [System.Formats.Asn1.AsnEncodingRules]::DER)
    $outer = $reader.ReadSequence()
    $data = $outer.ReadSequence()
    [void]$data.ReadObjectIdentifier()
    if ($data.HasData) { [void]$data.ReadEncodedValue() }
    if ($data.HasData) { throw 'Authenticode SPC indirect-data value has trailing fields' }
    $digestInfo = $outer.ReadSequence()
    $algorithm = $digestInfo.ReadSequence()
    $algorithmOid = $algorithm.ReadObjectIdentifier()
    while ($algorithm.HasData) { [void]$algorithm.ReadEncodedValue() }
    $digest = $digestInfo.ReadOctetString()
    if ($digestInfo.HasData -or $outer.HasData -or $reader.HasData) {
        throw 'Authenticode SPC indirect-data content has trailing fields'
    }
    [pscustomobject]@{
        AlgorithmOid = $algorithmOid
        Digest = [byte[]]$digest
    }
}

function Get-AuthenticodeHashAlgorithm {
    param([Parameter(Mandatory = $true)] [string]$AlgorithmOid)
    switch ($AlgorithmOid) {
        '1.3.14.3.2.26' { return [System.Security.Cryptography.HashAlgorithmName]::SHA1 }
        '2.16.840.1.101.3.4.2.1' { return [System.Security.Cryptography.HashAlgorithmName]::SHA256 }
        '2.16.840.1.101.3.4.2.2' { return [System.Security.Cryptography.HashAlgorithmName]::SHA384 }
        '2.16.840.1.101.3.4.2.3' { return [System.Security.Cryptography.HashAlgorithmName]::SHA512 }
        default { throw "Authenticode signature uses unsupported image digest algorithm: $AlgorithmOid" }
    }
}

function Get-AuthenticodeIntegrityProof {
    param(
        [Parameter(Mandatory = $true)] [string]$Path,
        [Parameter(Mandatory = $true)] [string]$ExpectedThumbprint
    )
    $bytes = [IO.File]::ReadAllBytes($Path)
    $layout = Get-AuthenticodePeLayout $bytes
    $expected = $ExpectedThumbprint.Trim().ToUpperInvariant()
    $tableEnd = $layout.CertificateTableOffset + $layout.CertificateTableSize
    $cursor = $layout.CertificateTableOffset
    $cmsCount = 0
    while ($cursor -lt $tableEnd) {
        if ($cursor + 8 -gt $tableEnd) { throw 'Authenticode certificate table entry header is truncated' }
        $entryLength = [long](Read-UInt32LittleEndian $bytes ([int]$cursor))
        if ($entryLength -lt 8 -or $cursor + $entryLength -gt $tableEnd) {
            throw 'Authenticode certificate table entry length is invalid'
        }
        $certificateType = Read-UInt16LittleEndian $bytes ([int]($cursor + 6))
        if ($certificateType -eq 2) {
            $payloadLength = [int]($entryLength - 8)
            $payload = [byte[]]$bytes[([int]($cursor + 8))..([int]($cursor + 8 + $payloadLength - 1))]
            $payloadReader = [System.Formats.Asn1.AsnReader]::new(
                $payload,
                [System.Formats.Asn1.AsnEncodingRules]::DER)
            $encodedCms = $payloadReader.ReadEncodedValue().ToArray()
            $cms = [System.Security.Cryptography.Pkcs.SignedCms]::new()
            $cms.Decode($encodedCms)
            $cms.CheckSignature($true)
            $spcDigest = Get-AuthenticodeSpcDigest $cms.ContentInfo.Content
            $hashAlgorithm = Get-AuthenticodeHashAlgorithm $spcDigest.AlgorithmOid
            $computedDigest = Get-AuthenticodeImageDigest $bytes $layout $hashAlgorithm
            if (-not ([Convert]::ToHexString($computedDigest).Equals(
                    [Convert]::ToHexString($spcDigest.Digest),
                    [StringComparison]::OrdinalIgnoreCase))) {
                throw 'Authenticode signed image digest does not match the PE content'
            }
            foreach ($signer in $cms.SignerInfos) {
                $certificate = $signer.Certificate
                if ($null -eq $certificate) { continue }
                $signerThumbprint = ([string]$certificate.Thumbprint).Trim().ToUpperInvariant()
                if ($signerThumbprint -eq $expected) {
                    return [pscustomobject]@{
                        Verification = 'signature-valid-independent-cryptographic-integrity'
                        IntegrityVerifier = 'signedcms-spc-indirect-data-authenticode-hash'
                        IntegrityStatus = 'Valid'
                        SignerCertificate = $certificate
                        SignerThumbprint = $signerThumbprint
                        DigestAlgorithm = $spcDigest.AlgorithmOid
                        SignedDigest = ([Convert]::ToHexString($spcDigest.Digest)).ToLowerInvariant()
                        ComputedDigest = ([Convert]::ToHexString($computedDigest)).ToLowerInvariant()
                    }
                }
            }
            $cmsCount++
        }
        $roundedLength = [long]([math]::Ceiling($entryLength / 8.0) * 8)
        if ($roundedLength -le 0) { throw 'Authenticode certificate table entry alignment is invalid' }
        $cursor += $roundedLength
    }
    if ($cursor -ne $tableEnd) {
        throw 'Authenticode certificate table alignment does not match its declared size'
    }
    if ($cmsCount -eq 0) { throw 'Authenticode certificate table has no PKCS#7 signature' }
    throw 'Authenticode PKCS#7 signature has no signer matching the expected thumbprint'
}
