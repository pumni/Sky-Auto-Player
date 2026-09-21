# scripts/test_v4_nsis_smoke_boundary.ps1
# Direct regression tests proving hermetic NSIS smoke isolation contract.
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

. (Join-Path $PSScriptRoot "v4_nsis_smoke_boundary.ps1")

function Assert-True($condition, $message) {
    if (-not $condition) {
        throw "Assertion failed: $message"
    }
}

function Assert-Equal($actual, $expected, $message) {
    if ($actual -ne $expected) {
        throw "Assertion failed: $message. Expected '$expected', got '$actual'."
    }
}

Write-Host "Running NSIS smoke boundary tests..."

# Test 1: Absent key stays absent after successful scope
Write-Host "  Test 1: Absent registry key remains absent after success..."
$testCustomKey1 = "HKCU:\Software\__test_sky_absent_success\Sky Auto Player"
$testCustomParent1 = "HKCU:\Software\__test_sky_absent_success"
if (Test-Path -LiteralPath $testCustomParent1) {
    Remove-Item -LiteralPath $testCustomParent1 -Recurse -Force
}

$customTargets1 = @([ordered]@{ Key = $testCustomKey1; Parent = $testCustomParent1 })
$scope1 = Enter-V4NsisSmokeScope -RegistryTargets $customTargets1
try {
    # Simulate installer writing location key
    New-Item -Path $testCustomKey1 -Force -Value "C:\fake\smoke\install" | Out-Null
    Set-ItemProperty -Path $testCustomKey1 -Name "Installer Language" -Value "1033"
    Assert-True (Test-Path -LiteralPath $testCustomKey1) "Simulated installer key was created"
} finally {
    Exit-V4NsisSmokeScope -Scope $scope1
}

Assert-True (-not (Test-Path -LiteralPath $testCustomKey1)) "Absent registry key was left behind after success"
Assert-True (-not (Test-Path -LiteralPath $testCustomParent1)) "Parent manufacturer key was left behind after success"
Write-Host "    PASS"

# Test 1b: FreshInstall neutralizes pre-existing monitored state before entry
Write-Host "  Test 1b: Fresh-install mode starts without historical registry state..."
$freshKey = "HKCU:\Software\__test_sky_fresh_mode\Sky Auto Player"
$freshParent = "HKCU:\Software\__test_sky_fresh_mode"
if (Test-Path -LiteralPath $freshParent) {
    Remove-Item -LiteralPath $freshParent -Recurse -Force
}
New-Item -Path $freshKey -Force -Value "C:\historical\install" | Out-Null
$freshTargets = @([ordered]@{ Key = $freshKey; Parent = $freshParent })
$freshScope = Enter-V4NsisSmokeScope -RegistryTargets $freshTargets -RegistryStateMode FreshInstall
try {
    Assert-True (-not (Test-Path -LiteralPath $freshKey)) "Fresh-install mode left historical registry state before installer execution"
    New-Item -Path $freshKey -Force -Value "C:\fresh\install" | Out-Null
} finally {
    Exit-V4NsisSmokeScope -Scope $freshScope
}
Assert-Equal (Get-Item -LiteralPath $freshKey).GetValue('') "C:\historical\install" "Fresh-install mode restored the historical registry snapshot"
Remove-Item -LiteralPath $freshParent -Recurse -Force
Write-Host "    PASS"

# Test 2: Absent registry key remains absent after injected failure
Write-Host "  Test 2: Absent registry key remains absent after injected failure..."
$testCustomKey = "HKCU:\Software\__test_sky_isolated_manu\Sky Auto Player"
$testCustomParent = "HKCU:\Software\__test_sky_isolated_manu"
if (Test-Path -LiteralPath $testCustomParent) {
    Remove-Item -LiteralPath $testCustomParent -Recurse -Force
}

$customTargets = @([ordered]@{ Key = $testCustomKey; Parent = $testCustomParent })
$customSnapshot = @{
    $testCustomKey = [ordered]@{
        KeyPath       = $testCustomKey
        ParentPath    = $testCustomParent
        ParentExisted = $false
        KeyExisted    = $false
        Properties    = [ordered]@{}
        SubKeyNames   = @()
    }
}

$failureTriggered = $false
try {
    try {
        New-Item -Path $testCustomKey -Force -Value "C:\throwaway\install" | Out-Null
        Set-ItemProperty -Path $testCustomKey -Name "Garbage" -Value "test"
        throw "Simulated harness crash during smoke"
    } finally {
        Restore-V4NsisRegistryState -Snapshots $customSnapshot
    }
} catch {
    if ($_.Exception.Message -match "Simulated harness crash") {
        $failureTriggered = $true
    } else {
        throw $_
    }
}

Assert-True $failureTriggered "Failure occurred as expected"
Assert-True (-not (Test-Path -LiteralPath $testCustomKey)) "Key should not exist after failed scope"
Assert-True (-not (Test-Path -LiteralPath $testCustomParent)) "Parent manufacturer key should be removed when empty and previously absent"
Write-Host "    PASS"

# Test 3: Pre-existing sentinel registry key restored exactly after success
Write-Host "  Test 3: Pre-existing sentinel restored after success..."
New-Item -Path $testCustomKey -Force -Value "C:\original\real\install" | Out-Null
Set-ItemProperty -Path $testCustomKey -Name "Installer Language" -Value "1033"
Set-ItemProperty -Path $testCustomKey -Name "SentinelDword" -Value 12345 -Type DWord

$sentinelSnapshot = @{
    $testCustomKey = [ordered]@{
        KeyPath       = $testCustomKey
        ParentPath    = $testCustomParent
        ParentExisted = $true
        KeyExisted    = $true
        Properties    = [ordered]@{
            '' = [ordered]@{ Value = "C:\original\real\install"; Kind = 'String' }
            'Installer Language' = [ordered]@{ Value = "1033"; Kind = 'String' }
            'SentinelDword' = [ordered]@{ Value = 12345; Kind = 'DWord' }
        }
        SubKeyNames   = @()
    }
}

# Now simulate smoke mutating it
Set-Item -LiteralPath $testCustomKey -Value "C:\throwaway\corrupted"
Set-ItemProperty -LiteralPath $testCustomKey -Name "CorruptedProp" -Value "polluted"
Remove-ItemProperty -LiteralPath $testCustomKey -Name "SentinelDword"

# Restore
Restore-V4NsisRegistryState -Snapshots $sentinelSnapshot

$restoredItem = Get-Item -LiteralPath $testCustomKey
Assert-Equal ($restoredItem.GetValue('')) "C:\original\real\install" "Default value restored"
Assert-Equal ($restoredItem.GetValue('Installer Language')) "1033" "String property restored"
Assert-Equal ($restoredItem.GetValue('SentinelDword')) 12345 "DWord property restored"
Assert-Equal ($restoredItem.GetValueKind('SentinelDword')) ([Microsoft.Win32.RegistryValueKind]::DWord) "DWord kind preserved"
Assert-True (-not ($restoredItem.GetValueNames() -contains "CorruptedProp")) "Corrupted property was removed"
Write-Host "    PASS"

# Test 4: Pre-existing sentinel restored after injected failure
Write-Host "  Test 4: Pre-existing sentinel restored after injected failure..."
$failureTriggered = $false
try {
    try {
        Set-Item -LiteralPath $testCustomKey -Value "C:\throwaway\failed_run"
        Set-ItemProperty -LiteralPath $testCustomKey -Name "FailureResidue" -Value "fail"
        throw "Simulated failure during installer run"
    } finally {
        Restore-V4NsisRegistryState -Snapshots $sentinelSnapshot
    }
} catch {
    if ($_.Exception.Message -match "Simulated failure") {
        $failureTriggered = $true
    } else {
        throw $_
    }
}

Assert-True $failureTriggered "Failure occurred as expected"
$restoredItem = Get-Item -LiteralPath $testCustomKey
Assert-Equal ($restoredItem.GetValue('')) "C:\original\real\install" "Default value restored on failure"
Assert-True (-not ($restoredItem.GetValueNames() -contains "FailureResidue")) "Failure residue was cleaned up"

# Cleanup test sentinel
Remove-Item -LiteralPath $testCustomParent -Recurse -Force
Write-Host "    PASS"

# Test 5: AppData isolation and environment variable restore
Write-Host "  Test 5: AppData isolation and environment restoration..."
$origEnv = "C:\original\appdata"
[Environment]::SetEnvironmentVariable('SKY_APP_DATA_ROOT', $origEnv, 'Process')
$appDataScope = Enter-V4NsisSmokeScope
try {
    $currentEnv = [Environment]::GetEnvironmentVariable('SKY_APP_DATA_ROOT', 'Process')
    Assert-True ($currentEnv -ne $origEnv) "SKY_APP_DATA_ROOT was isolated"
    Assert-True (Test-Path -LiteralPath $currentEnv) "Isolated appdata directory exists"
    $appDataDir = $currentEnv
} finally {
    Exit-V4NsisSmokeScope -Scope $appDataScope
}

Assert-Equal ([Environment]::GetEnvironmentVariable('SKY_APP_DATA_ROOT', 'Process')) $origEnv "Original SKY_APP_DATA_ROOT restored"
Assert-True (-not (Test-Path -LiteralPath $appDataDir)) "Temporary AppData directory was cleaned up"
Remove-Item Env:SKY_APP_DATA_ROOT -ErrorAction SilentlyContinue
Write-Host "    PASS"

# Test 6: Bounded retry and fail-closed directory cleanup
Write-Host "  Test 6: Bounded retry directory cleanup fail-closed detection..."
$testDir = Join-Path ([IO.Path]::GetTempPath()) ("sky-boundary-test-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $testDir -Force | Out-Null
$lockFile = Join-Path $testDir "locked.tmp"
$stream = [IO.File]::Open($lockFile, [IO.FileMode]::Create, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)

$cleanupFailed = $false
$cleanupError = ""
try {
    Remove-V4DirectoryWithRetry -Path $testDir -MaxAttempts 3 -DelayMilliseconds 50
} catch {
    $cleanupFailed = $true
    $cleanupError = $_.Exception.Message
} finally {
    $stream.Close()
    $stream.Dispose()
    Remove-Item -LiteralPath $testDir -Recurse -Force -ErrorAction SilentlyContinue
}

Assert-True $cleanupFailed "Remove-V4DirectoryWithRetry fails closed when directory is locked"
Assert-True ($cleanupError -match "path=") "Directory cleanup failure reports path"
Assert-True ($cleanupError -match "last_error=") "Directory cleanup failure reports last_error"
Assert-True ($cleanupError -match "residue=") "Directory cleanup failure reports residue"
Assert-True ($cleanupError -like "*locked.tmp*") "Directory cleanup residue lists locked file"
Write-Host "    PASS"

# Test 7: Injected failure in one cleanup stage does not skip subsequent cleanup stages
Write-Host "  Test 7: Attempt-all cleanup executes remaining stages after an earlier failure..."
$origEnv7 = "C:\original\appdata-test7"
[Environment]::SetEnvironmentVariable('SKY_APP_DATA_ROOT', $origEnv7, 'Process')
$installRoot7 = Join-Path ([IO.Path]::GetTempPath()) ("sky-boundary-install7-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $installRoot7 -Force | Out-Null
$scope7 = Enter-V4NsisSmokeScope -InstallRoot $installRoot7 -ManageInstallRootCleanup

# Inject an intentional registry snapshot corruption that causes Stage 2 to fail
$scope7.RegistrySnapshots = @{
    'HKCU:\Software\__nonexistent_test_key_fail_stage2' = [ordered]@{
        KeyPath       = 'HKCU:\Software\__nonexistent_test_key_fail_stage2'
        ParentPath    = 'HKCU:\Software'
        ParentExisted = $true
        KeyExisted    = $true # Claim it existed, but when it attempts to open it for writing with invalid subpath or throws
        Properties    = [ordered]@{
            'FailProp' = [ordered]@{ Value = "fail"; Kind = 'DWord' }
        }
        SubKeyNames   = @()
    }
}
# Delete the key if it exists so OpenSubKey fails or creates error
if (Test-Path -LiteralPath 'HKCU:\Software\__nonexistent_test_key_fail_stage2') {
    Remove-Item -LiteralPath 'HKCU:\Software\__nonexistent_test_key_fail_stage2' -Recurse -Force
}

$stageFailureThrown = $false
$thrownMessage = ""
try {
    # Exit scope with an original test error as well to verify context preservation
    Exit-V4NsisSmokeScope -Scope $scope7 -OriginalError ([System.InvalidOperationException]::new("Simulated original runner test error"))
} catch {
    $stageFailureThrown = $true
    $thrownMessage = $_.Exception.Message
}

Assert-True $stageFailureThrown "Exit-V4NsisSmokeScope must fail when a stage fails"
Assert-True ($thrownMessage -match "Exit-V4NsisSmokeScope failed with") "Aggregate error message reported"
Assert-True ($thrownMessage -match "Simulated original runner test error") "Original test error preserved in aggregate message"
# Crucial assertions: stages 3, 4, 5 must still have been attempted and succeeded!
Assert-Equal ([Environment]::GetEnvironmentVariable('SKY_APP_DATA_ROOT', 'Process')) $origEnv7 "Environment variable was restored despite stage 2 failure"
Assert-True (-not (Test-Path -LiteralPath $scope7.AppDataRoot)) "Throwaway AppData was cleaned up despite stage 2 failure"
Assert-True (-not (Test-Path -LiteralPath $installRoot7)) "InstallRoot was cleaned up despite stage 2 failure"
Remove-Item Env:SKY_APP_DATA_ROOT -ErrorAction SilentlyContinue
# Clean up the fake test key if created
if (Test-Path -LiteralPath 'HKCU:\Software\__nonexistent_test_key_fail_stage2') {
    Remove-Item -LiteralPath 'HKCU:\Software\__nonexistent_test_key_fail_stage2' -Recurse -Force
}
Write-Host "    PASS"

# Test 8: Default value exact kind restoration (e.g. ExpandString / String)
Write-Host "  Test 8: Default registry value restores with exact captured kind..."
$testCustomKey8 = "HKCU:\Software\__test_sky_default_kind\Sky Auto Player"
$testCustomParent8 = "HKCU:\Software\__test_sky_default_kind"
if (Test-Path -LiteralPath $testCustomParent8) {
    Remove-Item -LiteralPath $testCustomParent8 -Recurse -Force
}
New-Item -Path $testCustomKey8 -Force | Out-Null
$subPath8 = $testCustomKey8.Substring('HKCU:\'.Length)
$writable8 = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($subPath8, $true)
$writable8.SetValue('', '%SystemRoot%\SkyAutoPlayer', [Microsoft.Win32.RegistryValueKind]::ExpandString)
$writable8.Close()

$snap8 = Protect-V4NsisRegistryState -Targets @([ordered]@{ Key = $testCustomKey8; Parent = $testCustomParent8 })
# Mutate default value to plain String
$writable8 = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($subPath8, $true)
$writable8.SetValue('', 'C:\corrupted', [Microsoft.Win32.RegistryValueKind]::String)
$writable8.Close()

Restore-V4NsisRegistryState -Snapshots $snap8

$regItem8 = Get-Item -LiteralPath $testCustomKey8
Assert-Equal ($regItem8.GetValue('', $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)) "%SystemRoot%\SkyAutoPlayer" "Unexpanded default value string restored"
Assert-Equal ($regItem8.GetValueKind('')) ([Microsoft.Win32.RegistryValueKind]::ExpandString) "Default value kind ExpandString was preserved"

Remove-Item -LiteralPath $testCustomParent8 -Recurse -Force
Write-Host "    PASS"

# Test 9: Registry equivalence assertion detects residue and fails closed
Write-Host "  Test 9: Registry equivalence assertion fails closed on leftover residue..."
$testCustomKey9 = "HKCU:\Software\__test_sky_residue\Sky Auto Player"
$testCustomParent9 = "HKCU:\Software\__test_sky_residue"
if (Test-Path -LiteralPath $testCustomParent9) {
    Remove-Item -LiteralPath $testCustomParent9 -Recurse -Force
}
New-Item -Path $testCustomKey9 -Force | Out-Null

$mockSnapshot9 = @{
    $testCustomKey9 = [ordered]@{
        KeyPath       = $testCustomKey9
        ParentPath    = $testCustomParent9
        ParentExisted = $true
        KeyExisted    = $true
        Properties    = [ordered]@{} # Expected 0 properties
        SubKeyNames   = @()
    }
}

# Inject a leftover property
Set-ItemProperty -Path $testCustomKey9 -Name "LeftoverResidue" -Value "bad"

$residueCaught = $false
try {
    Assert-V4NsisRegistryEquivalence -Snapshots $mockSnapshot9
} catch {
    if ($_.Exception.Message -match "Registry residue detected") {
        $residueCaught = $true
    } else {
        throw $_
    }
}
Assert-True $residueCaught "Assert-V4NsisRegistryEquivalence must detect unexpected leftover properties"

Remove-Item -LiteralPath $testCustomParent9 -Recurse -Force
Write-Host "    PASS"

# Test 10: Tracked processes terminated by Exit-V4NsisSmokeScope
Write-Host "  Test 10: Tracked child processes terminated by Exit-V4NsisSmokeScope..."
$psPath = (Get-Command powershell.exe).Source
$childProc = Start-Process -FilePath $psPath -ArgumentList "-NoProfile", "-Command", "Start-Sleep -Seconds 30" -WindowStyle Hidden -PassThru
$scope10 = Enter-V4NsisSmokeScope
$scope10.TrackedProcesses.Add($childProc)
try {
    Assert-True (-not $childProc.HasExited) "Tracked process is running before Exit-V4NsisSmokeScope"
} finally {
    Exit-V4NsisSmokeScope -Scope $scope10
}
Assert-True $childProc.HasExited "Tracked process must be terminated after Exit-V4NsisSmokeScope"
Write-Host "    PASS"

# Load updater fixture process helpers from ci_tauri_update_e2e_core.ps1
$coreScriptPath = Join-Path $PSScriptRoot "ci_tauri_update_e2e_core.ps1"
$ast = [System.Management.Automation.Language.Parser]::ParseFile($coreScriptPath, [ref]$null, [ref]$null)
$targetFunctions = @('Test-ProcessPathUnderRoots', 'Stop-UpdaterFixtureProcesses')
$functions = $ast.FindAll({
    $args[0] -is [System.Management.Automation.Language.FunctionDefinitionAst] -and
    ($args[0].Name -in $targetFunctions)
}, $true)
foreach ($fn in $functions) {
    . ([scriptblock]::Create($fn.Extent.Text))
}

# Test 11: Pure path matching helper (Test-ProcessPathUnderRoots)
Write-Host "  Test 11: Pure path matching helper (Test-ProcessPathUnderRoots)..."
$testRoot = "C:\test\fixture\root"
$resolvedRoots = @($testRoot)

# executable under root => owned
Assert-True (Test-ProcessPathUnderRoots -Path "C:\test\fixture\root\app.exe" -ResolvedRoots $resolvedRoots) "Executable directly under root is owned"
Assert-True (Test-ProcessPathUnderRoots -Path "C:\test\fixture\root\sub\app.exe" -ResolvedRoots $resolvedRoots) "Executable in subfolder of root is owned"

# sibling prefix collision => not owned
Assert-True (-not (Test-ProcessPathUnderRoots -Path "C:\test\fixture\root-sibling\app.exe" -ResolvedRoots $resolvedRoots)) "Sibling prefix collision is not owned"

# executable outside root => not owned
Assert-True (-not (Test-ProcessPathUnderRoots -Path "C:\Windows\System32\cmd.exe" -ResolvedRoots $resolvedRoots)) "Executable outside root is not owned"
Assert-True (-not (Test-ProcessPathUnderRoots -Path "" -ResolvedRoots $resolvedRoots)) "Empty executable path is not owned"
Write-Host "    PASS"

# Test 12: Live fixture-root process sweep (Stop-UpdaterFixtureProcesses)
Write-Host "  Test 12: Live fixture-root process sweep (Stop-UpdaterFixtureProcesses)..."
$sweepTempRoot = Join-Path ([IO.Path]::GetTempPath()) ("sky-sweep-test-" + [guid]::NewGuid().ToString("N"))
$insideRoot = Join-Path $sweepTempRoot "inside"
$outsideRoot = Join-Path $sweepTempRoot "outside"
New-Item -ItemType Directory -Path $insideRoot, $outsideRoot -Force | Out-Null

$insideExe = Join-Path $insideRoot "inside_worker.exe"
$outsideExe = Join-Path $outsideRoot "outside_worker.exe"
Copy-Item -LiteralPath $psPath -Destination $insideExe
Copy-Item -LiteralPath $psPath -Destination $outsideExe

$insideProc = Start-Process -FilePath $insideExe -ArgumentList "-NoProfile", "-Command", "Start-Sleep -Seconds 30" -WindowStyle Hidden -PassThru
$outsideProc = Start-Process -FilePath $outsideExe -ArgumentList "-NoProfile", "-Command", "Start-Sleep -Seconds 30" -WindowStyle Hidden -PassThru

try {
    Assert-True (-not $insideProc.HasExited) "Inside process is running"
    Assert-True (-not $outsideProc.HasExited) "Outside process is running"

    Stop-UpdaterFixtureProcesses -Roots @($insideRoot)

    Assert-True $insideProc.HasExited "Inside process was stopped by sweep"
    Assert-True (-not $outsideProc.HasExited) "Outside process was not targeted by sweep"
} finally {
    if (-not $insideProc.HasExited) { Stop-Process -Id $insideProc.Id -Force -ErrorAction SilentlyContinue }
    if (-not $outsideProc.HasExited) { Stop-Process -Id $outsideProc.Id -Force -ErrorAction SilentlyContinue }
    Remove-Item -LiteralPath $sweepTempRoot -Recurse -Force -ErrorAction SilentlyContinue
}
Write-Host "    PASS"

Write-Host "All NSIS smoke boundary tests PASS."
