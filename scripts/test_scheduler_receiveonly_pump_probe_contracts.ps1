$ErrorActionPreference = 'Stop'

$probePath = Join-Path $PSScriptRoot 'scheduler_receiveonly_pump_probe.ps1'
$probeText = [System.IO.File]::ReadAllText($probePath)
$tokens = $null
$parseErrors = $null
[System.Management.Automation.Language.Parser]::ParseFile($probePath, [ref]$tokens, [ref]$parseErrors) | Out-Null
if ($parseErrors.Count -ne 0) {
    throw ('Probe parser failed: ' + (($parseErrors | ForEach-Object Message) -join '; '))
}
if ($probeText -match 'SetForegroundWindow|AttachThreadInput|SendInputNative|\.SendInput\s*\(') {
    throw 'Probe contains a focus bypass or SendInput call path'
}
if ($probeText -notmatch '\$Source\s+-eq\s+''SendInput''\s+-and\s+-not\s+\(\$CompileOnly\s+-or\s+\$SelfTest\)') {
    throw 'SendInput runtime guard is missing or no longer precedes probe startup'
}

$evidenceRoot = Join-Path $env:TEMP ('scheduler-pump-contract-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $evidenceRoot | Out-Null

$compileOutput = & pwsh -NoProfile -File $probePath -Pump Native -Source PostMessage -OutputRoot $evidenceRoot -CompileOnly 2>&1
$compileExit = $LASTEXITCODE
if ($compileExit -ne 0 -or ($compileOutput -join "`n") -notmatch 'COMPILE PASS') {
    throw "Probe compile contract failed (exit $compileExit): $($compileOutput -join ' ')"
}

$selfTestOutput = & pwsh -NoProfile -File $probePath -Pump Native -Source PostMessage -OutputRoot $evidenceRoot -SelfTest 2>&1
$selfTestExit = $LASTEXITCODE
if ($selfTestExit -ne 0 -or ($selfTestOutput -join "`n") -notmatch 'foreground state, source gate, SendInput block, and retrieval correlation: PASS') {
    throw "Probe state/correlation tests failed (exit $selfTestExit): $($selfTestOutput -join ' ')"
}

$blockedOutput = & pwsh -NoProfile -File $probePath -Pump Native -Source SendInput -OutputRoot $evidenceRoot 2>&1
$blockedExit = $LASTEXITCODE
if ($blockedExit -eq 0 -or ($blockedOutput -join "`n") -notmatch 'disabled before probe startup; no SendInput call was made') {
    throw "SendInput diagnostic was not rejected before startup (exit $blockedExit): $($blockedOutput -join ' ')"
}

Write-Output 'scheduler pump probe parser, compile, foreground/retrieval state tests, and zero-call SendInput guard: PASS'
