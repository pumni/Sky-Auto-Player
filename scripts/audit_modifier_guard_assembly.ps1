param(
    [Parameter(Mandatory)]
    [string]$SkyPlayerAssemblyPath,
    [Parameter(Mandatory)]
    [string]$Win32AssemblyPath
)

$ErrorActionPreference = 'Stop'
$skyAssembly = (Resolve-Path -LiteralPath $SkyPlayerAssemblyPath -ErrorAction Stop).Path
$win32Assembly = (Resolve-Path -LiteralPath $Win32AssemblyPath -ErrorAction Stop).Path
if ([IO.Path]::GetFileName($skyAssembly) -notmatch '^sky_player(?:-[0-9a-f]+)?\.s$') {
    throw "expected the optimized sky_player assembly, got $skyAssembly"
}
if ([IO.Path]::GetFileName($win32Assembly) -notmatch '^sky_dispatch_win32(?:-[0-9a-f]+)?\.s$') {
    throw "expected the optimized sky_dispatch_win32 assembly, got $win32Assembly"
}

function Find-FunctionBody([string[]]$Assembly, [string]$LabelPattern) {
    $starts = [System.Collections.Generic.List[int]]::new()
    for ($index = 0; $index -lt $Assembly.Count; $index++) {
        if ($Assembly[$index] -match $LabelPattern) {
            $starts.Add($index)
        }
    }
    if ($starts.Count -ne 1) {
        throw "expected exactly one assembly function matching '$LabelPattern', found $($starts.Count)"
    }
    $start = $starts[0]
    for ($index = $start + 1; $index -lt $Assembly.Count; $index++) {
        if ($Assembly[$index].Trim() -eq '.seh_endproc') {
            return ,@($Assembly[$start..$index])
        }
    }
    throw "no .seh_endproc found for '$LabelPattern'"
}

function Assert-FiveModifierQueries(
    [string[]]$Body,
    [string]$Scope,
    [switch]$AllowOtherCalls
) {
    $expected = @(0x5B, 0x5C, 0x11, 0x10, 0x12)
    $actual = [System.Collections.Generic.List[int]]::new()
    $zeroWindows = [System.Collections.Generic.List[bool]]::new()
    $otherCalls = [System.Collections.Generic.List[string]]::new()
    $nextVirtualKey = $null
    $zeroWindow = $false
    foreach ($line in $Body) {
        if ($line -match 'xorl\s+%ecx,\s*%ecx') {
            $zeroWindow = $true
        }
        if ($line -match 'movl\s+\$(\d+),\s*%edx') {
            $nextVirtualKey = [int]$Matches[1]
        }
        if ($line -match 'callq.*query_async_key_state') {
            if ($null -eq $nextVirtualKey) {
                throw "$Scope query call had no constant VK immediate"
            }
            $actual.Add($nextVirtualKey)
            $zeroWindows.Add($zeroWindow)
            $nextVirtualKey = $null
            $zeroWindow = $false
        } elseif ($line -match '\bcallq\b') {
            $otherCalls.Add($line.Trim())
        }
    }
    if (($actual -join ',') -ne ($expected -join ',')) {
        throw "$Scope modifier VK calls were '$($actual -join ',')', expected '$($expected -join ',')'"
    }
    if ($zeroWindows.Count -ne 5 -or @($zeroWindows | Where-Object { -not $_ }).Count -ne 0) {
        throw "$Scope did not pass a zero window handle for all five modifier queries"
    }
    if ($otherCalls.Count -ne 0 -and -not $AllowOtherCalls) {
        throw "$Scope made a non-query call inside the fixed modifier probe: $($otherCalls -join '; ')"
    }
}

$modifierSourcePath = Join-Path $PSScriptRoot '..\rust\crates\sky_dispatch_win32\src\input\modifier_guard.rs'
$modifierSource = Get-Content -LiteralPath $modifierSourcePath -Raw
$arrayStart = $modifierSource.IndexOf('const MODIFIER_VKS')
$arrayEnd = $modifierSource.IndexOf('];', $arrayStart)
if ($arrayStart -lt 0 -or $arrayEnd -lt 0) {
    throw 'fixed modifier VK array was not found in source'
}
$arraySource = $modifierSource.Substring($arrayStart, $arrayEnd - $arrayStart)
$sourceVks = @([regex]::Matches($arraySource, '\((VK_[A-Z]+),').ForEach({ $_.Groups[1].Value }))
$expectedNames = @('VK_LWIN', 'VK_RWIN', 'VK_CONTROL', 'VK_SHIFT', 'VK_MENU')
if (($sourceVks -join ',') -ne ($expectedNames -join ',')) {
    throw "source modifier VK order was '$($sourceVks -join ',')', expected '$($expectedNames -join ',')'"
}
if ([regex]::Matches($arraySource, '\(VK_[A-Z]+,').Count -ne 5) {
    throw 'source modifier VK array must contain exactly five entries'
}
if ($modifierSource -notmatch 'observe_modifier_keys_with\(\|virtual_key\| query_async_key_state\(0, virtual_key\)\)') {
    throw 'production modifier source no longer queries GetAsyncKeyState with a null window handle'
}

$skyLines = [IO.File]::ReadAllLines($skyAssembly)
$win32Lines = [IO.File]::ReadAllLines($win32Assembly)
$queryBody = Find-FunctionBody $skyLines 'modifier_guard26observe_modifier_keys_with.*:$'
$admissionBody = Find-FunctionBody $skyLines 'admission37modifier_guard_and_final_revalidation:$'
$preparedBody = Find-FunctionBody $skyLines 'prepared30dispatch_prepared_normal_frame:$'
$win32QueryBody = Find-FunctionBody $win32Lines 'physical21query_async_key_state:$'

Assert-FiveModifierQueries $queryBody 'compiled fixed modifier observer'
Assert-FiveModifierQueries $admissionBody 'compiled authored Down admission' -AllowOtherCalls

$queryCalls = @($queryBody | Where-Object { $_ -match '\bcallq\b' })
$forbiddenProbeCalls = @(
    $queryBody | Where-Object {
        $_ -match '\b(?:memcpy|memmove|alloc|free|lock|GetForegroundWindow|OpenProcess|GetProcessTimes|QueryPerformanceCounter|MapVirtualKey|GetKeyboardLayout)\b'
    }
)
if ($queryCalls.Count -ne 5 -or $forbiddenProbeCalls.Count -ne 0) {
    throw "compiled modifier observer call count/forbidden-call audit failed: calls=$($queryCalls.Count), forbidden=$($forbiddenProbeCalls.Count)"
}

$preparedProbeCalls = @(
    $preparedBody | Where-Object { $_ -match 'callq.*modifier_guard26observe_modifier_keys_with' }
)
$preparedSenderCalls = @(
    $preparedBody | Where-Object { $_ -match 'callq.*send_prepared_physical_packet_at_final_boundary' }
)
if ($preparedProbeCalls.Count -ne 1 -or $preparedSenderCalls.Count -ne 1) {
    throw "prepared Down assembly expected one modifier probe and one sender transaction; found probe=$($preparedProbeCalls.Count), sender=$($preparedSenderCalls.Count)"
}

$getAsyncCalls = @($win32QueryBody | Where-Object { $_ -match 'callq\s+\*__imp_GetAsyncKeyState' })
$win32Calls = @($win32QueryBody | Where-Object { $_ -match '\bcallq\b' })
if ($getAsyncCalls.Count -ne 1 -or $win32Calls.Count -ne 1) {
    throw "Win32 query wrapper must make exactly one GetAsyncKeyState call; imports=$($getAsyncCalls.Count), calls=$($win32Calls.Count)"
}

Write-Output "sky_player_assembly=$skyAssembly"
Write-Output "win32_assembly=$win32Assembly"
Write-Output "source_modifier_vk_order=$($sourceVks -join ',')"
Write-Output "compiled_observer_GetAsyncKeyState_calls=$($queryCalls.Count)"
Write-Output "compiled_authored_admission_modifier_calls=$(@($admissionBody | Where-Object { $_ -match 'callq.*query_async_key_state' }).Count)"
Write-Output "prepared_dispatch_modifier_probe_calls=$($preparedProbeCalls.Count)"
Write-Output "prepared_sender_transactions=$($preparedSenderCalls.Count)"
Write-Output "GetAsyncKeyState_import_calls_per_query_async_wrapper=$($getAsyncCalls.Count)"
Write-Output 'modifier probe allocation/mapping/foreground/process/QPC/lock/log calls=0'
Write-Output 'optimized modifier guard assembly audit: PASS'
