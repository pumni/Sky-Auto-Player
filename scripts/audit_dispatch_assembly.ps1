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

$targets = @(
    @{ Name = 'plan_next_dispatch_projected'; Fragment = 'plan_next_dispatch_projected:'; Policy = 'report' },
    @{ Name = 'physical_plan_from_view'; Fragment = 'physical_plan_from_view:'; Policy = 'report' },
    @{ Name = 'dispatch_due_from_plan'; Fragment = 'dispatch_due_from_plan:'; Policy = 'clean' },
    @{ Name = 'recover_missed_down_boundary'; Fragment = 'recover_missed_down_boundary:'; Policy = 'clean' },
    @{ Name = 'normal shipping caller'; Fragment = 'dispatch_loop8dispatch0B9_:'; Policy = 'report' }
)

function Find-FunctionBody([string]$fragment) {
    $start = -1
    for ($index = 0; $index -lt $lines.Count; $index++) {
        $line = $lines[$index]
        if ($line.Length -lt 400 -and $line.TrimEnd().EndsWith($fragment)) {
            $start = $index
            break
        }
    }
    if ($start -lt 0) {
        return $null
    }

    $end = -1
    for ($index = $start + 1; $index -lt $lines.Count; $index++) {
        if ($lines[$index].Trim() -eq '.seh_endproc') {
            $end = $index
            break
        }
    }
    if ($end -lt 0) {
        throw "No .seh_endproc found for $fragment"
    }
    return @($start, $end)
}

$failed = $false
Write-Output "assembly=$resolvedAssemblyPath"
foreach ($target in $targets) {
    $range = Find-FunctionBody $target.Fragment
    if ($null -eq $range) {
        if ($target.Policy -eq 'clean') {
            Write-Output "$($target.Name): NOT_FOUND (required clean target)"
            $failed = $true
        } else {
            Write-Output "$($target.Name): INLINED_OR_NOT_FOUND (report-only target)"
        }
        continue
    }

    $body = $lines[$range[0]..$range[1]]
    $instructionCount = @($body | Where-Object { $_ -match '^\s*[a-z][a-z0-9]*(?:\s|$)' }).Count
    $stackFrames = @(
        $body |
            Where-Object { $_ -match '\.seh_stackalloc\s+(\d+)' } |
            ForEach-Object { $Matches[1] }
    ) -join ','
    $memcpyCount = @($body | Where-Object { $_ -match '\bmemcpy\b' }).Count
    $memmoveCount = @($body | Where-Object { $_ -match '\bmemmove\b' }).Count
    $chkstkCount = @($body | Where-Object { $_ -match '\b__chkstk\b' }).Count
    $udivti3Count = @($body | Where-Object { $_ -match '\b__udivti3\b' }).Count
    $divti3Count = @($body | Where-Object { $_ -match '\b__divti3\b' }).Count
    $divisionInstructionCount = @(
        $body | Where-Object { $_ -match '^\s*(?:u?div|idiv)[bwlq]\s' }
    ).Count
    $copySizes = @(
        $body |
            Where-Object { $_ -match 'movl\s+\$(\d+),\s+%r8d' } |
            ForEach-Object { $Matches[1] }
    ) -join ','
    Write-Output (
        '{0}: lines={1}-{2} instructions={3} seh_stackalloc=[{4}] memcpy={5} memmove={6} __chkstk={7} __udivti3={8} __divti3={9} div_instructions={10} copy_size_immediates=[{11}]' -f
        $target.Name, ($range[0] + 1), ($range[1] + 1), $instructionCount, $stackFrames,
        $memcpyCount, $memmoveCount, $chkstkCount, $udivti3Count, $divti3Count,
        $divisionInstructionCount, $copySizes
    )

    if ($target.Policy -eq 'clean' -and (
            $memcpyCount -gt 0 -or
            $memmoveCount -gt 0 -or
            $chkstkCount -gt 0 -or
            $udivti3Count -gt 0 -or
            $divti3Count -gt 0 -or
            $divisionInstructionCount -gt 0
        )) {
        $failed = $true
    }
}

# Gate A deliberately does not require a retained Rust helper symbol. ThinLTO
# may inline the normal precision suffix into the dispatch-loop caller. Audit
# the optimized caller for the actual prepared sender handoff and report any
# retained helper call only as compiler output, not as a source-level contract.
$callerRange = Find-FunctionBody 'dispatch_loop8dispatch0B9_:'
if ($null -ne $callerRange) {
    $callerBody = $lines[$callerRange[0]..$callerRange[1]]
    $helperCalls = @(
        $callerBody | Where-Object {
            $_ -match 'callq.*send_prepared_normal_precision_frame'
        }
    )
    $preparedSenderCalls = @(
        $callerBody | Where-Object {
            $_ -match 'callq.*send_prepared_physical_packet_at_final_boundary'
        }
    )
    $panicRmwInstructions = @(
        $callerBody | Where-Object { $_ -match '\b(?:xchg|cmpxchg)\w*\b' }
    )
    $powerConsumeRmwInstructions = @(
        $callerBody | Where-Object { $_ -match '\bcmpxchg\w*\b' }
    )
    Write-Output (
        'normal precision caller: retained_helper_call_refs={0} prepared_sender_calls={1} atomic_rmw_instructions={2} power_consume_rmw_candidates={3}' -f
        $helperCalls.Count, $preparedSenderCalls.Count, $panicRmwInstructions.Count,
        $powerConsumeRmwInstructions.Count
    )
    if ($helperCalls.Count -gt 0) {
        Write-Output 'normal precision caller: compiler retained an out-of-line helper call; inspect this caller region as the shipping suffix'
    } else {
        Write-Output 'normal precision caller: no retained send_prepared_normal_precision_frame call (inlined or eliminated)'
    }
}

if ($failed) {
    exit 1
}
