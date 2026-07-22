#Requires -Modules Pester

# Import the updater module
. (Join-Path $PSScriptRoot '..\updater.ps1')

# Setup test environment using script: scope
$script:TestConfigDir = Join-Path $env:TEMP ('sky-updater-test-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force -Path $script:TestConfigDir | Out-Null
$script:TestConfigPath = Join-Path $script:TestConfigDir 'config.json'
$script:ConfigPath = $script:TestConfigPath
$script:InstallRoot = $script:TestConfigDir
$script:ExePath = Join-Path $script:TestConfigDir 'Sky-Auto-Player.exe'
$script:LogDir = Join-Path $env:TEMP 'Sky-Auto-Player-Test'
$script:LogFile = Join-Path $script:LogDir 'updater.log'
New-Item -ItemType File -Force -Path $script:ExePath | Out-Null

function Run-Tests {
    # Test 1: Update existing fields
    $config = @{
        theme = 'aurora'
        update = @{
            auto_check = $true
            channel = 'stable'
            skip_version = ''
            check_interval_s = 86400
            last_check_ts = 0
            last_error_ts = 0
            last_notified_version = ''
            legacy_old_dir_sweep_pending = $false
        }
    } | ConvertTo-Json -Depth 10
    $config | Out-File -FilePath $script:TestConfigPath -Encoding UTF8

    Write-UpdateFields -LastCheckTs 1718200000 -LastNotifiedVersion '2.4.0'
    $result = Get-Content -Raw -LiteralPath $script:TestConfigPath | ConvertFrom-Json
    if ($result.update.last_check_ts -eq 1718200000 -and $result.update.last_notified_version -eq '2.4.0' -and $result.theme -eq 'aurora') {
        Write-Host 'Test 1 PASSED: updates existing fields'
    } else {
        Write-Host 'Test 1 FAILED'
        $result | ConvertTo-Json -Depth 10
    }

    # Test 2: Creates update object if missing
    Remove-Item -Force $script:TestConfigPath -ErrorAction SilentlyContinue
    $config = @{ theme = 'aurora' } | ConvertTo-Json -Depth 10
    $config | Out-File -FilePath $script:TestConfigPath -Encoding UTF8
    Write-UpdateFields -LastCheckTs 12345 -LastNotifiedVersion '1.0.0'
    $result = Get-Content -Raw -LiteralPath $script:TestConfigPath | ConvertFrom-Json
    if ($result.update.last_check_ts -eq 12345 -and $result.update.last_notified_version -eq '1.0.0') {
        Write-Host 'Test 2 PASSED: creates update object'
    } else {
        Write-Host 'Test 2 FAILED'
        $result | ConvertTo-Json -Depth 10
    }

    # Test 3: Preserves unknown keys
    Remove-Item -Force $script:TestConfigPath -ErrorAction SilentlyContinue
    $config = @{
        theme = 'aurora'
        custom_user_field = 'should survive'
        update = @{ auto_check = $true; last_check_ts = 0; last_notified_version = '' }
    } | ConvertTo-Json -Depth 10
    $config | Out-File -FilePath $script:TestConfigPath -Encoding UTF8
    Write-UpdateFields -LastCheckTs 999 -LastNotifiedVersion 'x.y.z'
    $result = Get-Content -Raw -LiteralPath $script:TestConfigPath | ConvertFrom-Json
    if ($result.custom_user_field -eq 'should survive') {
        Write-Host 'Test 3 PASSED: preserves unknown keys'
    } else {
        Write-Host 'Test 3 FAILED'
        $result | ConvertTo-Json -Depth 10
    }

    # Test 4: UTF-8 without BOM
    Remove-Item -Force $script:TestConfigPath -ErrorAction SilentlyContinue
    $config = @{ update = @{ last_check_ts = 0; last_notified_version = '' } } | ConvertTo-Json -Depth 10
    $config | Out-File -FilePath $script:TestConfigPath -Encoding UTF8
    Write-UpdateFields -LastCheckTs 1 -LastNotifiedVersion '1'
    $bytes = [System.IO.File]::ReadAllBytes($script:TestConfigPath)
    $hasBom = ($bytes.Length -ge 3) -and ($bytes[0] -eq 0xEF) -and ($bytes[1] -eq 0xBB) -and ($bytes[2] -eq 0xBF)
    if (-not $hasBom) { Write-Host 'Test 4 PASSED: UTF-8 no BOM' } else { Write-Host 'Test 4 FAILED: has BOM' }

    # Test 5: Empty config
    Remove-Item -Force $script:TestConfigPath -ErrorAction SilentlyContinue
    '{}' | Out-File -FilePath $script:TestConfigPath -Encoding UTF8
    Write-UpdateFields -LastCheckTs 42 -LastNotifiedVersion 'test'
    $result = Get-Content -Raw -LiteralPath $script:TestConfigPath | ConvertFrom-Json
    if ($result.update.last_check_ts -eq 42 -and $result.update.last_notified_version -eq 'test') {
        Write-Host 'Test 5 PASSED: empty config'
    } else {
        Write-Host 'Test 5 FAILED'
        $result | ConvertTo-Json -Depth 10
    }

    # Cleanup
    Remove-Item -Recurse -Force $script:TestConfigDir -ErrorAction SilentlyContinue
    Write-Host 'All manual tests completed.'
}

Run-Tests