[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [ValidateSet('P0', 'P1', 'P2', 'P3', 'P4', 'P5')]
    [string]$Phase,
    [Parameter(Mandatory)]
    [ValidatePattern('^[0-9a-fA-F]{40}$')]
    [string]$HeadSha,
    [Parameter(Mandatory)]
    [string]$EvidenceRoot
)

$ErrorActionPreference = 'Stop'
$utf8NoBom = [System.Text.UTF8Encoding]::new($false)
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$tempRoot = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath())
$publisherTemp = Join-Path $tempRoot ('rt-r3-publisher-' + [guid]::NewGuid().ToString('N'))
$tempRepo = Join-Path $publisherTemp 'repo'
$packageRoot = Join-Path $publisherTemp 'package'
$stagingRoot = Join-Path $publisherTemp 'staging'
$evidenceRunDirectory = $null
$published = $false

function Invoke-GitQuiet {
    param([Parameter(Mandatory)][string]$WorkingDirectory, [Parameter(Mandatory)][string[]]$Arguments)
    $output = & git -C $WorkingDirectory @Arguments 2>&1
    $exitCode = $LASTEXITCODE
    if ($exitCode -ne 0) {
        $verb = if ($Arguments.Count -gt 0) { $Arguments[0] } else { 'git' }
        throw "git $verb failed with exit code $exitCode; authenticated evidence push is not confirmed"
    }
    return @($output | ForEach-Object { [string]$_ })
}

function Write-JsonFile([string]$Path, [object]$Value) {
    $json = ConvertTo-Json -InputObject $Value -Depth 20
    $jsonLf = $json.Replace("`r`n", "`n").Replace("`r", "`n")
    [System.IO.File]::WriteAllText($Path, $jsonLf + "`n", $utf8NoBom)
}

function Get-DeclaredOutputFiles([string]$Root, [object[]]$Entries) {
    $rootFull = [System.IO.Path]::GetFullPath($Root).TrimEnd('\') + '\'
    foreach ($entry in $Entries) {
        if (-not $entry.path -or $entry.path -match '(^|[\\/])\.\.([\\/]|$)' -or [System.IO.Path]::IsPathRooted([string]$entry.path)) {
            throw "Runner declared an unsafe evidence path: $($entry.path)"
        }
        $candidate = [System.IO.Path]::GetFullPath((Join-Path $Root ([string]$entry.path)))
        if (-not $candidate.StartsWith($rootFull, [System.StringComparison]::OrdinalIgnoreCase)) {
            throw "Runner output escaped EvidenceRoot: $($entry.path)"
        }
        if (-not (Test-Path -LiteralPath $candidate -PathType Leaf)) { throw "Declared evidence file is missing: $($entry.path)" }
        $file = Get-Item -LiteralPath $candidate -Force
        if (($file.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) { throw "Refusing reparse-point evidence: $($entry.path)" }
        $hash = (Get-FileHash -LiteralPath $candidate -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($entry.sha256 -and $entry.sha256.ToLowerInvariant() -ne $hash) { throw "Evidence changed after runner declaration: $($entry.path)" }
        [ordered]@{ source_path = $candidate; relative_path = ([string]$entry.path).Replace('\', '/'); size_bytes = $file.Length; sha256 = $hash }
    }
}

function Write-Checksums([string]$Root, [string[]]$RelativePaths, [string]$Destination) {
    $lines = @(
        foreach ($relative in $RelativePaths) {
        $path = Join-Path $Root ($relative.Replace('/', '\'))
        $hash = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
        "$hash  $relative"
        }
    )
    $checksumText = [string]::Join("`n", [string[]]$lines) + "`n"
    [System.IO.File]::WriteAllText($Destination, $checksumText, $utf8NoBom)
}

function Get-Sha256Bytes([byte[]]$Bytes) {
    $sha256 = [System.Security.Cryptography.SHA256]::Create()
    try {
        return [BitConverter]::ToString($sha256.ComputeHash($Bytes)).Replace('-', '').ToLowerInvariant()
    } finally {
        $sha256.Dispose()
    }
}

function Get-GitBlobBytes([string]$WorkingDirectory, [string]$ObjectPath) {
    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = 'git'
    [void]$startInfo.ArgumentList.Add('-C')
    [void]$startInfo.ArgumentList.Add($WorkingDirectory)
    [void]$startInfo.ArgumentList.Add('cat-file')
    [void]$startInfo.ArgumentList.Add('blob')
    [void]$startInfo.ArgumentList.Add($ObjectPath)
    $startInfo.UseShellExecute = $false
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true

    $process = [System.Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    $output = [System.IO.MemoryStream]::new()
    try {
        if (-not $process.Start()) { throw 'Could not start git cat-file for committed evidence verification' }
        $copyOutput = $process.StandardOutput.BaseStream.CopyToAsync($output)
        $readError = $process.StandardError.ReadToEndAsync()
        $process.WaitForExit()
        [void]$copyOutput.GetAwaiter().GetResult()
        $errorText = $readError.GetAwaiter().GetResult()
        if ($process.ExitCode -ne 0) {
            throw "git cat-file failed for $ObjectPath with exit code $($process.ExitCode): $errorText"
        }
        return ,$output.ToArray()
    } finally {
        $output.Dispose()
        $process.Dispose()
    }
}

function Assert-CommittedEvidenceBytes(
    [string]$WorkingDirectory,
    [string]$Commit,
    [string]$RunId,
    [string]$RunDirectory,
    [object[]]$ArchiveParts
) {
    $manifestPath = Join-Path $RunDirectory 'manifest.json'
    $checksumsPath = Join-Path $RunDirectory 'SHA256SUMS.txt'
    $checksumText = [System.IO.File]::ReadAllText($checksumsPath, $utf8NoBom)
    if ($checksumText.Contains("`r") -or -not $checksumText.EndsWith("`n") -or $checksumText.StartsWith([string][char]0xFEFF)) {
        throw 'Generated outer SHA256SUMS.txt is not UTF-8 without BOM with fixed LF and trailing LF'
    }

    $expectedHashes = @{}
    foreach ($line in $checksumText.Split("`n", [System.StringSplitOptions]::RemoveEmptyEntries)) {
        if ($line -notmatch '^([0-9a-f]{64})  (.+)$') { throw "Invalid outer checksum row: $line" }
        $expectedHashes[$Matches[2]] = $Matches[1]
    }

    $manifestBytes = Get-GitBlobBytes -WorkingDirectory $WorkingDirectory -ObjectPath "${Commit}:runs/$RunId/manifest.json"
    $manifestHash = Get-Sha256Bytes -Bytes $manifestBytes
    $manifestFile = Get-Item -LiteralPath $manifestPath
    $expectedManifestHash = $expectedHashes['manifest.json']
    if (-not $expectedManifestHash -or
        $manifestBytes.Length -ne $manifestFile.Length -or
        $manifestHash -ne $expectedManifestHash -or
        $manifestHash -ne (Get-FileHash -LiteralPath $manifestPath -Algorithm SHA256).Hash.ToLowerInvariant()) {
        throw 'Committed outer manifest size/SHA256 differs from its generated bytes or SHA256SUMS.txt'
    }
    if ($manifestBytes.Length -eq 0 -or
        ($manifestBytes.Length -ge 3 -and $manifestBytes[0] -eq 239 -and $manifestBytes[1] -eq 187 -and $manifestBytes[2] -eq 191) -or
        [Array]::IndexOf($manifestBytes, [byte]13) -ge 0 -or
        $manifestBytes[$manifestBytes.Length - 1] -ne 10) {
        throw 'Committed outer manifest is not UTF-8 without BOM with fixed LF and trailing LF'
    }

    $checksumsBytes = Get-GitBlobBytes -WorkingDirectory $WorkingDirectory -ObjectPath "${Commit}:runs/$RunId/SHA256SUMS.txt"
    $workingChecksumsBytes = [System.IO.File]::ReadAllBytes($checksumsPath)
    if ($checksumsBytes.Length -ne $workingChecksumsBytes.Length -or
        (Get-Sha256Bytes -Bytes $checksumsBytes) -ne (Get-Sha256Bytes -Bytes $workingChecksumsBytes)) {
        throw 'Committed outer SHA256SUMS.txt differs from its generated fixed-LF bytes'
    }

    foreach ($part in $ArchiveParts) {
        $relativePath = [string]$part.path
        $objectPath = "${Commit}:runs/$RunId/$relativePath"
        $blobBytes = Get-GitBlobBytes -WorkingDirectory $WorkingDirectory -ObjectPath $objectPath
        $blobHash = Get-Sha256Bytes -Bytes $blobBytes
        $expectedHash = $expectedHashes[$relativePath]
        if (-not $expectedHash -or
            $blobBytes.Length -ne [long]$part.size_bytes -or
            $blobHash -ne $expectedHash -or
            $blobHash -ne [string]$part.sha256) {
            throw "Committed archive part size/SHA256 mismatch: $relativePath"
        }
    }

    return $manifestHash
}

function Split-Archive([string]$ArchivePath, [string]$DestinationDirectory, [long]$MaximumBytes) {
    $archiveInfo = Get-Item -LiteralPath $ArchivePath
    if ($archiveInfo.Length -le $MaximumBytes) {
        $single = Join-Path $DestinationDirectory 'evidence.zip'
        Copy-Item -LiteralPath $ArchivePath -Destination $single
        return @([ordered]@{ path = 'evidence.zip'; size_bytes = (Get-Item -LiteralPath $single).Length; sha256 = (Get-FileHash -LiteralPath $single -Algorithm SHA256).Hash.ToLowerInvariant() })
    }

    $parts = [System.Collections.Generic.List[object]]::new()
    $inputStream = [System.IO.File]::OpenRead($ArchivePath)
    try {
        $buffer = [byte[]]::new(1MB)
        $partIndex = 1
        while ($inputStream.Position -lt $inputStream.Length) {
            $partPath = Join-Path $DestinationDirectory ('evidence.zip.part{0:D4}' -f $partIndex)
            $partStream = [System.IO.File]::Create($partPath)
            $partBytes = 0L
            try {
                while ($partBytes -lt $MaximumBytes -and $inputStream.Position -lt $inputStream.Length) {
                    $remaining = [int][Math]::Min($buffer.Length, $MaximumBytes - $partBytes)
                    $read = $inputStream.Read($buffer, 0, $remaining)
                    if ($read -le 0) { break }
                    $partStream.Write($buffer, 0, $read)
                    $partBytes += $read
                }
            } finally { $partStream.Dispose() }
            $parts.Add([ordered]@{ path = [System.IO.Path]::GetFileName($partPath); size_bytes = $partBytes; sha256 = (Get-FileHash -LiteralPath $partPath -Algorithm SHA256).Hash.ToLowerInvariant() })
            $partIndex++
        }
    } finally { $inputStream.Dispose() }
    return @($parts)
}

try {
    $headSha = $HeadSha.ToLowerInvariant()
    $rootPath = if ([System.IO.Path]::IsPathRooted($EvidenceRoot)) {
        [System.IO.Path]::GetFullPath($EvidenceRoot)
    } else {
        [System.IO.Path]::GetFullPath((Join-Path $repoRoot $EvidenceRoot))
    }
    if (-not (Test-Path -LiteralPath $rootPath -PathType Container)) { throw 'EvidenceRoot does not exist' }
    $runnerManifestPath = Join-Path $rootPath 'runner-outputs.json'
    if (-not (Test-Path -LiteralPath $runnerManifestPath -PathType Leaf)) { throw 'EvidenceRoot has no runner-outputs.json' }
    $runnerManifest = Get-Content -LiteralPath $runnerManifestPath -Raw | ConvertFrom-Json
    if ($runnerManifest.status -ne 'COMPLETED_CHARACTERIZATION') { throw 'Runner output is not a completed characterization' }
    if ($runnerManifest.source_revision.ToLowerInvariant() -ne $headSha) { throw 'HeadSha does not match the runner source revision' }
    if ($Phase -ne 'P0' -or $runnerManifest.stage -ne 'baseline') { throw 'This P0 publisher route accepts only a completed baseline run' }
    if ($runnerManifest.run_id -notmatch '^[A-Za-z0-9._-]+$') { throw 'Runner run_id contains unsafe path characters' }
    $runId = [string]$runnerManifest.run_id
    $evidenceBranch = 'rt-r3/evidence/p0-' + $headSha.Substring(0, 12)
    $currentHead = (& git -C $repoRoot rev-parse --verify HEAD).Trim()
    if ($LASTEXITCODE -ne 0 -or $currentHead.ToLowerInvariant() -ne $headSha) { throw 'Implementation worktree HEAD differs from HeadSha' }
    $currentBranch = (& git -C $repoRoot branch --show-current).Trim()
    if ($LASTEXITCODE -ne 0 -or $currentBranch -ne 'rt-r3/p0-baseline') { throw 'P0 publishing requires the fixed implementation branch rt-r3/p0-baseline' }
    $trackedStatus = @(& git -C $repoRoot status --porcelain --untracked-files=all)
    if ($LASTEXITCODE -ne 0 -or $trackedStatus.Count -ne 0) { throw 'P0 publishing requires a clean implementation worktree' }

    $originUrl = (& git -C $repoRoot remote get-url origin).Trim()
    if ($LASTEXITCODE -ne 0 -or $originUrl -notmatch '(github\.com[:/]pumni/Sky-Auto-Player)(\.git)?$') { throw 'Origin is not pumni/Sky-Auto-Player on GitHub' }
    $declared = @(Get-DeclaredOutputFiles -Root $rootPath -Entries @($runnerManifest.declared_outputs))
    if ($declared.Count -eq 0) { throw 'Runner declared no evidence outputs' }

    [System.IO.Directory]::CreateDirectory($publisherTemp) | Out-Null
    [System.IO.Directory]::CreateDirectory($tempRepo) | Out-Null
    [System.IO.Directory]::CreateDirectory($packageRoot) | Out-Null
    $packageFilesRoot = Join-Path $packageRoot 'files'
    [System.IO.Directory]::CreateDirectory($packageFilesRoot) | Out-Null
    foreach ($file in $declared) {
        $destination = Join-Path $packageFilesRoot ($file.relative_path.Replace('/', '\'))
        [System.IO.Directory]::CreateDirectory([System.IO.Path]::GetDirectoryName($destination)) | Out-Null
        Copy-Item -LiteralPath $file.source_path -Destination $destination
    }
    Copy-Item -LiteralPath $runnerManifestPath -Destination (Join-Path $packageRoot 'runner-outputs.json')

    $innerFileManifest = @($declared | ForEach-Object { [ordered]@{ path = "files/$($_.relative_path)"; size_bytes = $_.size_bytes; sha256 = $_.sha256 } })
    $innerManifest = [ordered]@{
        schema_version = 1
        phase = $Phase
        run_id = $runId
        source_revision = $runnerManifest.source_revision
        runtime_revision = $runnerManifest.runtime_revision
        head_sha = $headSha
        executable_sha256 = $runnerManifest.executable_sha256
        transport_kind = 'deterministic-mock'
        declared_outputs = $innerFileManifest
    }
    Write-JsonFile (Join-Path $packageRoot 'manifest.json') $innerManifest
    $innerChecksumPaths = @('manifest.json', 'runner-outputs.json') + @($innerFileManifest | ForEach-Object { $_.path })
    Write-Checksums -Root $packageRoot -RelativePaths $innerChecksumPaths -Destination (Join-Path $packageRoot 'SHA256SUMS.txt')
    $archivePath = Join-Path $publisherTemp 'evidence.zip'
    Compress-Archive -Path (Join-Path $packageRoot '*') -DestinationPath $archivePath -CompressionLevel Optimal

    $evidenceRunDirectory = Join-Path (Join-Path $stagingRoot $runId) 'run'
    $archiveDestination = Join-Path $evidenceRunDirectory 'archive'
    [System.IO.Directory]::CreateDirectory($archiveDestination) | Out-Null
    $archiveParts = @(Split-Archive -ArchivePath $archivePath -DestinationDirectory $archiveDestination -MaximumBytes (8MB))
    $outerManifest = [ordered]@{
        schema_version = 1
        phase = $Phase
        run_id = $runId
        evidence_branch = $evidenceBranch
        source_revision = $runnerManifest.source_revision
        runtime_revision = $runnerManifest.runtime_revision
        head_sha = $headSha
        executable_sha256 = $runnerManifest.executable_sha256
        runner_outputs_sha256 = (Get-FileHash -LiteralPath $runnerManifestPath -Algorithm SHA256).Hash.ToLowerInvariant()
        raw_output_count = $declared.Count
        raw_outputs = $innerFileManifest
        archive_format = 'ZIP; concatenate archive parts in numeric order when split'
        archive_parts = @($archiveParts | ForEach-Object { [ordered]@{ path = "archive/$($_.path)"; size_bytes = $_.size_bytes; sha256 = $_.sha256 } })
        archive_total_bytes = (Get-Item -LiteralPath $archivePath).Length
    }
    Write-JsonFile (Join-Path $evidenceRunDirectory 'manifest.json') $outerManifest
    $checksumPaths = @('manifest.json') + @($archiveParts | ForEach-Object { "archive/$($_.path)" })
    Write-Checksums -Root $evidenceRunDirectory -RelativePaths $checksumPaths -Destination (Join-Path $evidenceRunDirectory 'SHA256SUMS.txt')

    $originDirectory = 'origin'
    [void](Invoke-GitQuiet -WorkingDirectory $tempRepo -Arguments @('init', '--quiet'))
    [void](Invoke-GitQuiet -WorkingDirectory $tempRepo -Arguments @('remote', 'add', $originDirectory, $originUrl))
    $remoteRef = @(Invoke-GitQuiet -WorkingDirectory $tempRepo -Arguments @('ls-remote', '--heads', $originDirectory, $evidenceBranch))
    if ($remoteRef.Count -gt 0 -and $remoteRef[0] -match '^([0-9a-f]{40})\s+refs/heads/') {
        [void](Invoke-GitQuiet -WorkingDirectory $tempRepo -Arguments @('fetch', '--no-tags', $originDirectory, "refs/heads/${evidenceBranch}:refs/remotes/origin/${evidenceBranch}"))
        [void](Invoke-GitQuiet -WorkingDirectory $tempRepo -Arguments @('checkout', '--quiet', '--no-track', '-b', $evidenceBranch, "refs/remotes/origin/$evidenceBranch"))
    } else {
        [void](Invoke-GitQuiet -WorkingDirectory $tempRepo -Arguments @('fetch', '--no-tags', $originDirectory, 'refs/heads/main:refs/remotes/origin/main'))
        [void](Invoke-GitQuiet -WorkingDirectory $tempRepo -Arguments @('checkout', '--quiet', '--no-track', '-b', $evidenceBranch, 'refs/remotes/origin/main'))
    }
    if (Test-Path -LiteralPath (Join-Path (Join-Path $tempRepo 'runs') $runId)) { throw "Refusing to overwrite existing evidence run_id: $runId" }
    $destinationRun = Join-Path (Join-Path $tempRepo 'runs') $runId
    [System.IO.Directory]::CreateDirectory([System.IO.Path]::GetDirectoryName($destinationRun)) | Out-Null
    Move-Item -LiteralPath $evidenceRunDirectory -Destination $destinationRun
    [void](Invoke-GitQuiet -WorkingDirectory $tempRepo -Arguments @('add', '--', "runs/$runId"))
    [void](Invoke-GitQuiet -WorkingDirectory $tempRepo -Arguments @('commit', '--quiet', '-m', "docs(rt): publish $Phase evidence $runId"))
    $evidenceCommit = (@(Invoke-GitQuiet -WorkingDirectory $tempRepo -Arguments @('rev-parse', '--verify', 'HEAD')) | Select-Object -First 1).Trim()
    if ($evidenceCommit -notmatch '^[0-9a-f]{40}$') { throw 'Evidence commit could not be resolved before push' }
    $verifiedManifestSha256 = Assert-CommittedEvidenceBytes `
        -WorkingDirectory $tempRepo `
        -Commit $evidenceCommit `
        -RunId $runId `
        -RunDirectory $destinationRun `
        -ArchiveParts @($outerManifest.archive_parts)
    [void](Invoke-GitQuiet -WorkingDirectory $tempRepo -Arguments @('push', $originDirectory, "HEAD:refs/heads/$evidenceBranch"))
    $published = $true

    $result = [ordered]@{
        status = 'PUBLISHED'
        phase = $Phase
        run_id = $runId
        evidence_branch = $evidenceBranch
        evidence_commit = $evidenceCommit
        source_head = $headSha
        runtime_revision = $runnerManifest.runtime_revision
        evidence_url = "https://github.com/pumni/Sky-Auto-Player/tree/$evidenceCommit/runs/$runId"
        manifest_url = "https://github.com/pumni/Sky-Auto-Player/blob/$evidenceCommit/runs/$runId/manifest.json"
        manifest_sha256 = $verifiedManifestSha256
        archive_parts = @($outerManifest.archive_parts)
    }
    Write-Output (ConvertTo-Json -InputObject $result -Depth 10 -Compress)
} catch {
    Write-Error "BLOCKED-EVIDENCE: $($_.Exception.Message)"
    exit 1
} finally {
    $publisherFull = [System.IO.Path]::GetFullPath($publisherTemp)
    $tempRootFull = [System.IO.Path]::GetFullPath($tempRoot).TrimEnd('\') + '\'
    if ($publisherFull.StartsWith($tempRootFull, [System.StringComparison]::OrdinalIgnoreCase) -and [System.IO.Path]::GetFileName($publisherFull).StartsWith('rt-r3-publisher-', [System.StringComparison]::Ordinal)) {
        Remove-Item -LiteralPath $publisherFull -Recurse -Force -ErrorAction SilentlyContinue
    }
}
