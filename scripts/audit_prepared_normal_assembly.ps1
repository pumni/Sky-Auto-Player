param(
    [string]$AssemblyPath = ''
)

if ([string]::IsNullOrWhiteSpace($AssemblyPath)) {
    $assemblyDirectory = Join-Path $PSScriptRoot '..\rust\target\dist\deps'
    $candidates = @(
        Get-ChildItem -LiteralPath $assemblyDirectory -Filter 'sky_player*.s' -File |
            Where-Object { $_.Name -match '^sky_player(?:-[0-9a-f]+)?\.s$' }
    )
    if ($candidates.Count -ne 1) {
        throw "expected exactly one authoritative sky_player dist assembly in $assemblyDirectory, found $($candidates.Count)"
    }
    $AssemblyPath = $candidates[0].FullName
}

$resolvedAssemblyPath = Resolve-Path -LiteralPath $AssemblyPath -ErrorAction Stop
if ($resolvedAssemblyPath.Path -notmatch '[\\/]sky_player(?:-[0-9a-f]+)?\.s$') {
    throw "assembly audit must inspect authoritative sky_player output, got $resolvedAssemblyPath"
}
$lines = Get-Content -LiteralPath $resolvedAssemblyPath

function Find-FunctionBody([string]$fragment) {
    $start = -1
    for ($index = 0; $index -lt $lines.Count; $index++) {
        if ($lines[$index].Length -lt 400 -and $lines[$index].TrimEnd().EndsWith($fragment)) {
            $start = $index
            break
        }
    }
    if ($start -lt 0) {
        return $null
    }
    for ($index = $start + 1; $index -lt $lines.Count; $index++) {
        if ($lines[$index].Trim() -eq '.seh_endproc') {
            return @($start, $index)
        }
    }
    throw "No .seh_endproc found for $fragment"
}

$range = Find-FunctionBody 'dispatch_loop8dispatch0B9_:'
if ($null -eq $range) {
    throw 'prepared normal shipping caller was not emitted'
}
$body = $lines[$range[0]..$range[1]]
$senderIndices = @(
    for ($index = 0; $index -lt $body.Count; $index++) {
        if ($body[$index] -match 'callq.*send_prepared_physical_packet_at_final_boundary') {
            $index
        }
    }
)
if ($senderIndices.Count -ne 1) {
    throw "expected exactly one full prepared sender transaction in the shipping caller, found $($senderIndices.Count)"
}

# ThinLTO inlines the normal precision helper into a large dispatch caller.
# The reproducible Phase 3 scope is the bounded optimized suffix ending at
# the one healthy prepared sender handoff. The unrelated legacy caller body is
# intentionally outside this gate.
$senderIndex = $senderIndices[0]
$suffixStart = [Math]::Max(0, $senderIndex - 96)
$suffix = @($body[$suffixStart..$senderIndex])

$forbiddenCalls = @(
    'plan_next_dispatch',
    'dispatch_due_from_plan',
    'recover_missed_down_boundary',
    'commit_prepared',
    'commit_pending',
    'prepare_current_authored_packet',
    'pending_release',
    'coordinator',
    'planner',
    'schedule',
    'prepare_',
    'packet_',
    'PreparedPhysicalPacket',
    'alloc',
    'free',
    'lock',
    'Mutex'
)
$forbiddenCallMatches = @(
    $suffix | Where-Object {
        if ($_ -notmatch 'callq') {
            return $false
        }
        if ($_ -match 'send_prepared_physical_packet_at_final_boundary') {
            return $false
        }
        foreach ($token in $forbiddenCalls) {
            if ($_ -match [Regex]::Escape($token)) {
                return $true
            }
        }
        return $false
    }
)
$copyMatches = @($suffix | Where-Object { $_ -match '\b(?:memcpy|memmove|__chkstk|__udivti3|__divti3)\b' })
$helperMatches = @($body | Where-Object { $_ -match 'callq.*send_prepared_normal_precision_frame' })

Write-Output "assembly=$resolvedAssemblyPath"
Write-Output "scope=dispatch_loop8dispatch0B9_ suffix_lines=$($suffixStart + $range[0] + 1)-$($senderIndex + $range[0] + 1)"
Write-Output "normal_prepared_sender_calls=$($senderIndices.Count)"
Write-Output "retained_precision_helper_calls=$($helperMatches.Count)"
Write-Output "scoped_forbidden_calls=$($forbiddenCallMatches.Count)"
Write-Output "scoped_copy_or_division_symbols=$($copyMatches.Count)"

if ($helperMatches.Count -ne 0 -or $forbiddenCallMatches.Count -ne 0 -or $copyMatches.Count -ne 0) {
    throw 'scoped optimized normal-prepared audit failed'
}
Write-Output 'scoped optimized normal-prepared audit: PASS'
