# Offline test-only AB/BA process pairing. No SendInput or Git writes.
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$ExpectedHead,
    [ValidateSet('smoke','full')][string]$Scope = 'smoke',
    [ValidateRange(5,20)][int]$Pairs = 5,
    [string]$EvidenceDirectory = (Join-Path $env:TEMP (
        'sap-compact-ab-' + (Get-Date -Format 'yyyyMMdd-HHmmss')
    ))
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$rust = Join-Path $repo 'rust'

# On a detached worktree, 'git branch --show-current' outputs no line.
# Calling .Trim() on its PowerShell $null result incorrectly rejected the
# required validation configuration before any corpus/trial could run.
# 'rev-parse --abbrev-ref HEAD' produces literal HEAD when detached.
# Capture command output as arrays and check native exits before indexing.
$observedHead = @(git -C $repo rev-parse --verify HEAD)
$headExit = $LASTEXITCODE
if ($headExit -ne 0 -or $observedHead.Count -ne 1 -or
    $observedHead[0].Trim() -ne $ExpectedHead) {
    throw 'Candidate HEAD mismatch or git rev-parse failed'
}

$observedRef = @(git -C $repo rev-parse --abbrev-ref HEAD)
$refExit = $LASTEXITCODE
if ($refExit -ne 0 -or $observedRef.Count -ne 1 -or
    $observedRef[0].Trim() -ne 'HEAD') {
    throw 'Validation worktree must be detached at HEAD'
}

$trackedChanges = @(git -C $repo status --porcelain=v1)
$statusExit = $LASTEXITCODE
if ($statusExit -ne 0 -or $trackedChanges.Count -ne 0) {
    throw 'Validation worktree must be clean'
}
$temp = [IO.Path]::GetFullPath($env:TEMP).TrimEnd('\')
$out = [IO.Path]::GetFullPath($EvidenceDirectory)
if (-not $out.StartsWith($temp + '\', [StringComparison]::OrdinalIgnoreCase)) {
    throw 'Evidence must be inside TEMP'
}
New-Item -ItemType Directory -Force -Path $out | Out-Null
Push-Location $rust
try {
    $buildArguments = @('build','--locked','--release','-p','sky_player',
        '--features','test-support','--example','rt_prepared_process_probe',
        '--example','rt_corpus_packet_probe')
    & cargo @buildArguments
    if ($LASTEXITCODE -ne 0) { throw 'Offline probe compilation failed' }
}
finally { Pop-Location }
$exe = Join-Path $rust 'target\release\examples\rt_prepared_process_probe.exe'
$corpusExe = Join-Path $rust 'target\release\examples\rt_corpus_packet_probe.exe'
if (-not (Test-Path $exe) -or -not (Test-Path $corpusExe)) {
    throw 'Offline example executables missing'
}
$corpusFile = Join-Path $out 'corpus-control.json'
& $corpusExe --output $corpusFile
if ($LASTEXITCODE -ne 0) { throw 'Corpus control failed' }
$corpus = Get-Content -LiteralPath $corpusFile -Raw | ConvertFrom-Json -Depth 100
$expected = @(0,63324,23810,14698,4958,284,42,2,0,0,0,0,0,0,0,0)
if ($corpus.source_revision -ne $ExpectedHead -or
    $corpus.manifest_song_count -ne 112 -or
    $corpus.accounted_song_count -ne 112 -or
    $corpus.outcome_counts.prepared -ne 110 -or
    $corpus.outcome_counts.unsupported -ne 2 -or
    (@($corpus.prepared_packet_histogram) -join ',') -ne ($expected -join ',')) {
    throw '112-song control differs from qualified baseline'
}
$songs = @($corpus.songs | Where-Object { $_.evidence.outcome -eq 'prepared' })
if ($songs.Count -ne 110) { throw 'Expected 110 supported songs' }
if ($Scope -eq 'smoke') { $songs = @($songs[0],$songs[54],$songs[109]) }
$workloads = [Collections.Generic.List[object]]::new()
foreach ($song in $songs) {
    $workloads.Add([pscustomobject]@{
        key = ('song:' + $song.id); kind = 'song'; id = $song.id
        generations = 0; width = 0
    })
}
$counts = if ($Scope -eq 'full') { @(100,1000,10000) } else { @(100,1000) }
foreach ($count in $counts) {
    foreach ($width in @(1,15)) {
        $workloads.Add([pscustomobject]@{
            key = ('synthetic:' + $count + ':width:' + $width)
            kind = 'synthetic'; id = ''; generations = $count; width = $width
        })
    }
}
$rows = [Collections.Generic.List[object]]::new()
$deltas = [Collections.Generic.List[object]]::new()
$seq = 0
foreach ($w in $workloads) {
    $control = if ($w.kind -eq 'song') {
        $corpus.songs | Where-Object { $_.id -eq $w.id } | Select-Object -First 1
    } else { $null }
    foreach ($stage in @('cold','warm')) {
        foreach ($pair in 1..$Pairs) {
            $order = if ($pair % 2 -eq 1) {
                @('inline','compact')
            } else {
                @('compact','inline')
            }
            $matched = @{}
            foreach ($mode in $order) {
                $seq++
                $file = Join-Path $out ('trial-{0:D5}.json' -f $seq)
                $warm = if ($stage -eq 'warm') { '1' } else { '0' }
                $argv = @('--variant',$mode,'--warmup',$warm,'--output',$file)
                if ($w.kind -eq 'song') { $argv += @('--song-id',$w.id) }
                else {
                    $argv += @('--synthetic-generations',[string]$w.generations,
                        '--width',[string]$w.width)
                }
                & $exe @argv
                if ($LASTEXITCODE -ne 0) { throw "Child trial $seq failed" }
                $sample = Get-Content -LiteralPath $file -Raw | ConvertFrom-Json -Depth 100
                if ($sample.schema -ne 1 -or
                    $sample.source_revision -ne $ExpectedHead -or
                    $sample.manifest_sha256 -ne $corpus.manifest_sha256 -or
                    $sample.variant -ne $mode -or $sample.workload -ne $w.key -or
                    $sample.warmup_in_same_process -ne ($warm -eq '1')) {
                    throw "Invalid trial identity $seq"
                }
                if ($sample.prepared.entry_count -ne (
                        $sample.prepared.physical_packet_count +
                        $sample.prepared.metadata_entry_count
                    ) -or $sample.prepared.physical_packet_count -ne (
                        (@($sample.prepared.packet_histogram) | Measure-Object -Sum).Sum
                    )) { throw "Invalid trial structure $seq" }
                $matched[$mode] = $sample
                $mem = $sample.measurements.while_owner_held_process
                $rows.Add([pscustomobject]@{
                    sequence = $seq; workload = $w.key; stage = $stage
                    pair = $pair; order = ($order -join '-'); variant = $mode
                    prep_ns = [long]$sample.measurements.prepare_reset_ns
                    held_private_commit_bytes = [long]$mem.private_commit_bytes
                    held_working_set_bytes = [long]$mem.working_set_bytes
                    lifetime_peak_private_bytes =
                        [long]$mem.peak_private_commit_bytes_since_process_start
                    lifetime_peak_working_set_bytes =
                        [long]$mem.peak_working_set_bytes_since_process_start
                    rust_alloc_calls =
                        [long]$sample.measurements.rust_global_allocator_alloc_or_realloc_calls
                    rust_requested_bytes =
                        [long]$sample.measurements.rust_global_allocator_requested_bytes
                    reserved_stream_bytes =
                        [long]$sample.prepared.reserved_stream_buffer_bytes
                    file = (Split-Path $file -Leaf)
                })
            }
            $a = $matched.inline
            $b = $matched.compact
            if ($a.prepared.entry_count -ne $b.prepared.entry_count -or
                $a.prepared.physical_packet_count -ne $b.prepared.physical_packet_count -or
                $a.prepared.metadata_entry_count -ne $b.prepared.metadata_entry_count -or
                $a.prepared.input_events -ne $b.prepared.input_events -or
                $a.prepared.reset_coordinator_heap_payload_bytes -ne
                    $b.prepared.reset_coordinator_heap_payload_bytes -or
                (@($a.prepared.packet_histogram) -join ',') -ne
                    (@($b.prepared.packet_histogram) -join ',')) {
                throw "Pair data mismatch workload=$($w.key) pair=$pair"
            }
            if ($null -ne $control -and (
                $a.prepared.physical_packet_count -ne
                    $control.evidence.physical_packet_count -or
                $a.prepared.input_events -ne $control.evidence.input_events -or
                (@($a.prepared.packet_histogram) -join ',') -ne
                    (@($control.evidence.packet_histogram) -join ',')
            )) { throw "Song differs from corpus control $($w.key)" }
            $deltas.Add([pscustomobject]@{
                workload = $w.key; stage = $stage; pair = $pair
                order = ($order -join '-')
                compact_minus_inline_prep_ns =
                    ([long]$b.measurements.prepare_reset_ns -
                        [long]$a.measurements.prepare_reset_ns)
                compact_minus_inline_private_commit_bytes =
                    ([long]$b.measurements.while_owner_held_process.private_commit_bytes -
                        [long]$a.measurements.while_owner_held_process.private_commit_bytes)
                compact_minus_inline_working_set_bytes =
                    ([long]$b.measurements.while_owner_held_process.working_set_bytes -
                        [long]$a.measurements.while_owner_held_process.working_set_bytes)
                compact_minus_inline_rust_alloc_calls =
                    ([long]$b.measurements.rust_global_allocator_alloc_or_realloc_calls -
                        [long]$a.measurements.rust_global_allocator_alloc_or_realloc_calls)
            })
        }
    }
}
$rows | Export-Csv -LiteralPath (Join-Path $out 'trials.csv') -NoTypeInformation
$deltas | Export-Csv -LiteralPath (Join-Path $out 'paired-deltas.csv') -NoTypeInformation
[ordered]@{
    schema = 1
    source_revision = $ExpectedHead
    scope = $Scope
    pairs = $Pairs
    workload_count = $workloads.Count
    child_process_count = $seq
    order = 'interleaved AB/BA'
    manifest_sha256 = $corpus.manifest_sha256
    exe_sha256 = (Get-FileHash -Algorithm SHA256 $exe).Hash
    rustc = (& rustc --version)
    cpu = (Get-CimInstance Win32_Processor | Select-Object -First 1 -ExpandProperty Name)
    power_plan = (& powercfg /getactivescheme)
    warnings = @(
        'PrivateUsage is process private commit, working set includes shared resident pages.',
        'PeakPagefileUsage and peak working set cover the whole process, including compilation and warmup.',
        'Rust GlobalAlloc counts exclude native/CRT allocations.',
        'Warm stage repeats the same representation first; it does not control OS cold-cache state.',
        'No game/SendInput/receiver timing or production compact sender is involved.'
    )
} | ConvertTo-Json -Depth 8 |
    Set-Content -LiteralPath (Join-Path $out 'study-manifest.json') -Encoding utf8
Write-Host "Finished $seq isolated process samples at $out"
