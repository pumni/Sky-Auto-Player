param(
    [string]$AssemblyPath = (Join-Path $PSScriptRoot '..\rust\target\dist\deps\sky_player_rs.s')
)

$resolvedAssemblyPath = Resolve-Path -LiteralPath $AssemblyPath -ErrorAction Stop
$lines = Get-Content -LiteralPath $resolvedAssemblyPath

$targets = @(
    @{ Name = 'plan_next_dispatch_projected'; Fragment = 'plan_next_dispatch_projected:'; Policy = 'report' },
    @{ Name = 'physical_plan_from_view'; Fragment = 'physical_plan_from_view:'; Policy = 'report' },
    @{ Name = 'dispatch_due_from_plan'; Fragment = 'dispatch_due_from_plan:'; Policy = 'clean' },
    @{ Name = 'recover_missed_down_boundary'; Fragment = 'recover_missed_down_boundary:'; Policy = 'clean' },
    @{ Name = 'dispatch_loop caller'; Fragment = 'dispatch_loop8dispatch0B9_:'; Policy = 'report' }
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

if ($failed) {
    exit 1
}
