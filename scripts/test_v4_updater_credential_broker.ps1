# scripts/test_v4_updater_credential_broker.ps1
# Regression tests for Windows Credential Manager broker, provisioning scripts, and BuildCandidate boundary.
# Governed by docs/v4-release-execution-topology.md, docs/v4-updater-key-custody.md, and SECURITY.md.

[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if (-not [System.OperatingSystem]::IsWindows()) {
    throw "Credential broker regression tests require Windows platform"
}

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
. (Join-Path $PSScriptRoot "v4_updater_credential_broker.ps1")

function Fail([string]$Message) {
    throw "FAILED: $Message"
}

function Assert-NoSecretInText([string]$Secret, [string]$Text, [string]$Context) {
    if (-not [string]::IsNullOrEmpty($Secret) -and $Text.Contains($Secret)) {
        Fail "Secret material was emitted in output during: $Context"
    }
}

$testTarget = "SkyAutoPlayer/V4UpdaterUnitTest_" + [guid]::NewGuid().ToString("N")
$testSentinel = "V4_BENIGN_SENTINEL_" + [guid]::NewGuid().ToString("N")
$fixtureRoot = $null

try {
    Write-Host "Starting V4 updater credential broker tests..."

    # =========================================================================
    # Test 1: ABSENT fails closed
    # =========================================================================
    Write-Host "Test 1: ABSENT fails closed..."
    if (Test-V4UpdaterProductionCredential -Target $testTarget) {
        Fail "Test target should not exist before test"
    }
    $absentFailedClosed = $false
    try {
        $null = Get-V4UpdaterProductionCredential -Target $testTarget
    } catch {
        $absentFailedClosed = $true
        if ($_.Exception.Message -notmatch "absent") {
            Fail "Absent credential error message did not indicate absence: $($_.Exception.Message)"
        }
    }
    if (-not $absentFailedClosed) {
        Fail "Get-V4UpdaterProductionCredential did not fail closed on absent credential"
    }
    Write-Host "Test 1: PASS"

    # =========================================================================
    # Test 2: CRED_PERSIST_SESSION credential is readable
    # =========================================================================
    Write-Host "Test 2: CRED_PERSIST_SESSION credential is readable..."
    [SkyAutoPlayer.V4UpdaterCredentialBroker]::WriteSessionCredential($testTarget, $testSentinel)
    if (-not (Test-V4UpdaterProductionCredential -Target $testTarget)) {
        Fail "Test target should be present after WriteSessionCredential"
    }
    $readBack = Get-V4UpdaterProductionCredential -Target $testTarget
    if ($readBack -ne $testSentinel) {
        Fail "Read credential does not match written sentinel"
    }
    Write-Host "Test 2: PASS"

    # =========================================================================
    # Test 3: Empty credential blob is rejected
    # =========================================================================
    Write-Host "Test 3: Empty credential blob is rejected..."
    [SkyAutoPlayer.V4UpdaterCredentialBroker]::WriteRawCredential($testTarget, [byte[]]@(), 1)
    if (Test-V4UpdaterProductionCredential -Target $testTarget) {
        Fail "Test-V4UpdaterProductionCredential should report false for empty credential blob"
    }
    $emptyFailedClosed = $false
    try {
        $null = Get-V4UpdaterProductionCredential -Target $testTarget
    } catch {
        $emptyFailedClosed = $true
        if ($_.Exception.Message -notmatch "empty") {
            Fail "Empty credential error message did not indicate empty blob: $($_.Exception.Message)"
        }
    }
    if (-not $emptyFailedClosed) {
        Fail "Get-V4UpdaterProductionCredential did not fail closed on empty credential blob"
    }
    Write-Host "Test 3: PASS"

    # =========================================================================
    # Test 4: Deletion works and is idempotent
    # =========================================================================
    Write-Host "Test 4: Deletion works and is idempotent..."
    [SkyAutoPlayer.V4UpdaterCredentialBroker]::WriteSessionCredential($testTarget, $testSentinel)
    Remove-V4UpdaterProductionCredential -Target $testTarget
    if (Test-V4UpdaterProductionCredential -Target $testTarget) {
        Fail "Test target should not be present after Remove-V4UpdaterProductionCredential"
    }
    # Second removal should succeed without error (idempotent)
    Remove-V4UpdaterProductionCredential -Target $testTarget
    Write-Host "Test 4: PASS"

    # =========================================================================
    # Test 5: Secret material is never printed by broker actions
    # =========================================================================
    Write-Host "Test 5: Secret material is never printed by broker actions..."
    [SkyAutoPlayer.V4UpdaterCredentialBroker]::WriteSessionCredential($testTarget, $testSentinel)

    $readOutput = & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
        -File (Join-Path $PSScriptRoot "v4_updater_credential_broker.ps1") `
        -Action Read -BrokerTarget $testTarget 2>&1 | Out-String
    Assert-NoSecretInText -Secret $testSentinel -Text $readOutput -Context "Broker -Action Read"
    if ($readOutput -notmatch "\[PASS\]") {
        Fail "Broker -Action Read did not report sanitized PASS"
    }

    $deleteOutput = & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
        -File (Join-Path $PSScriptRoot "v4_updater_credential_broker.ps1") `
        -Action Delete -BrokerTarget $testTarget 2>&1 | Out-String
    Assert-NoSecretInText -Secret $testSentinel -Text $deleteOutput -Context "Broker -Action Delete"
    if ($deleteOutput -notmatch "\[PASS\]") {
        Fail "Broker -Action Delete did not report sanitized PASS"
    }
    Write-Host "Test 5: PASS"

    # =========================================================================
    # Test 6: Operator provisioning script (set_v4_updater_session_credential.ps1)
    # =========================================================================
    Write-Host "Test 6: Operator provisioning script contract..."
    # Empty input rejected
    $emptyCommand = "`$sec = [System.Security.SecureString]::new(); & '$($repoRoot.Replace('\', '/'))/scripts/set_v4_updater_session_credential.ps1' -TestSecureString `$sec -Target '$testTarget'; exit `$LASTEXITCODE"
    $emptyOutput = & pwsh -NoProfile -Command $emptyCommand 2>&1 | Out-String
    $emptyExitCode = $LASTEXITCODE
    if ($emptyExitCode -eq 0 -or $emptyOutput -notmatch "\[FAIL\] Passphrase input cannot be empty") {
        Fail "set_v4_updater_session_credential did not reject empty input (exit=$emptyExitCode, out=$emptyOutput)"
    }

    # Successful provisioning
    $setCommand = "`$sec = ConvertTo-SecureString '$testSentinel' -AsPlainText -Force; & '$($repoRoot.Replace('\', '/'))/scripts/set_v4_updater_session_credential.ps1' -TestSecureString `$sec -Target '$testTarget'; exit `$LASTEXITCODE"
    $setOutput = & pwsh -NoProfile -Command $setCommand 2>&1 | Out-String
    $setExitCode = $LASTEXITCODE
    Assert-NoSecretInText -Secret $testSentinel -Text $setOutput -Context "Provisioning script"
    if ($setExitCode -ne 0 -or $setOutput -notmatch "\[PASS\]") {
        Fail "set_v4_updater_session_credential did not report PASS (exit=$setExitCode, out=$setOutput)"
    }
    if (-not (Test-V4UpdaterProductionCredential -Target $testTarget)) {
        Fail "Target was not provisioned in Credential Manager. setOutput: $setOutput"
    }
    $retrieved = Get-V4UpdaterProductionCredential -Target $testTarget
    if ($retrieved -ne $testSentinel) {
        Fail "Provisioned credential value does not match sentinel"
    }

    # Cleanup script
    $removeOutput = & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
        -File (Join-Path $PSScriptRoot "remove_v4_updater_session_credential.ps1") `
        -Target $testTarget 2>&1 | Out-String
    $removeExitCode = $LASTEXITCODE
    Assert-NoSecretInText -Secret $testSentinel -Text $removeOutput -Context "Cleanup script"
    if ($removeExitCode -ne 0 -or $removeOutput -notmatch "\[PASS\]") {
        Fail "remove_v4_updater_session_credential did not report PASS (exit=$removeExitCode, out=$removeOutput)"
    }
    if (Test-V4UpdaterProductionCredential -Target $testTarget) {
        Fail "Target was not removed after remove_v4_updater_session_credential"
    }
    Write-Host "Test 6: PASS"

    # =========================================================================
    # Test 7: BuildCandidate integration (shims, env scoping, CLI secrecy, cleanup)
    # =========================================================================
    Write-Host "Test 7: BuildCandidate boundary integration with shims..."
    $fixtureRoot = Join-Path ([IO.Path]::GetTempPath()) ("sky-v4-buildcandidate-test-" + [guid]::NewGuid().ToString("N"))
    $mockLogPath = Join-Path $fixtureRoot "orchestrator-call.log"
    $mockEnvPath = Join-Path $fixtureRoot "orchestrator-env.log"
    $mockOrchestratorPath = Join-Path $fixtureRoot "mock_orchestrator.ps1"

    New-Item -ItemType Directory -Path $fixtureRoot -Force | Out-Null

    # Mock orchestrator that records its command-line arguments and process environment
    $mockScript = @"
param(
    [string]`$ExpectedSourceSha,
    [string]`$Version,
    [string]`$Channel,
    [string]`$UpdaterPrivateKeyPath,
    [string]`$UpdaterPasswordEnv = "TAURI_SIGNING_PRIVATE_KEY_PASSWORD"
)
Set-StrictMode -Version Latest
`$ErrorActionPreference = 'Stop'

# Record CLI arguments string
`$cliArgs = `$MyInvocation.Line
Set-Content -LiteralPath '$($mockLogPath.Replace('\', '/'))' -Value `$cliArgs -Encoding UTF8

# Record received password env
`$receivedPassword = [Environment]::GetEnvironmentVariable(`$UpdaterPasswordEnv, 'Process')
Set-Content -LiteralPath '$($mockEnvPath.Replace('\', '/'))' -Value `$receivedPassword -Encoding UTF8

if (`$env:SKY_MOCK_ORCHESTRATOR_FAIL -eq '1') {
    exit 42
}
exit 0
"@
    Set-Content -LiteralPath $mockOrchestratorPath -Value $mockScript -Encoding UTF8

    # Integration test helper mimicking Invoke-BuildCandidate broker wrapper
    function Invoke-TestBuildCandidateBroker(
        [string]$Target,
        [scriptblock]$Action
    ) {
        $prevPassword = [Environment]::GetEnvironmentVariable("TAURI_SIGNING_PRIVATE_KEY_PASSWORD", "Process")
        $credential = $null
        try {
            $credential = Get-V4UpdaterProductionCredential -Target $Target
            [Environment]::SetEnvironmentVariable("TAURI_SIGNING_PRIVATE_KEY_PASSWORD", $credential, "Process")
            & $Action
        } finally {
            if ($null -ne $prevPassword) {
                [Environment]::SetEnvironmentVariable("TAURI_SIGNING_PRIVATE_KEY_PASSWORD", $prevPassword, "Process")
            } else {
                [Environment]::SetEnvironmentVariable("TAURI_SIGNING_PRIVATE_KEY_PASSWORD", $null, "Process")
            }
            try {
                Remove-V4UpdaterProductionCredential -Target $Target
            } catch {
                Write-Warning "Cleanup returned: $($_.Exception.Message)"
            }
            $credential = $null
            Remove-Variable credential -ErrorAction SilentlyContinue
        }
    }

    # Case A: Success path
    [SkyAutoPlayer.V4UpdaterCredentialBroker]::WriteSessionCredential($testTarget, $testSentinel)
    $priorProcessEnv = [Environment]::GetEnvironmentVariable("TAURI_SIGNING_PRIVATE_KEY_PASSWORD", "Process")
    if (-not [string]::IsNullOrEmpty($priorProcessEnv)) {
        Fail "Prior process environment was unexpectedly non-empty"
    }

    Invoke-TestBuildCandidateBroker -Target $testTarget -Action {
        & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
            -File $mockOrchestratorPath `
            -ExpectedSourceSha "0123456789abcdef0123456789abcdef01234567" `
            -Version "4.0.0-rc.1" `
            -Channel "beta" `
            -UpdaterPrivateKeyPath "C:\path\to\key.key"
        if ($LASTEXITCODE -ne 0) { throw "Mock orchestrator failed" }
    }

    # Assertions for Case A:
    $recordedArgs = Get-Content -LiteralPath $mockLogPath -Raw
    Assert-NoSecretInText -Secret $testSentinel -Text $recordedArgs -Context "Orchestrator CLI arguments"
    $recordedEnv = Get-Content -LiteralPath $mockEnvPath -Raw
    if ($recordedEnv.Trim() -ne $testSentinel) {
        Fail "Mock orchestrator did not receive expected password environment"
    }
    $postProcessEnv = [Environment]::GetEnvironmentVariable("TAURI_SIGNING_PRIVATE_KEY_PASSWORD", "Process")
    if (-not [string]::IsNullOrEmpty($postProcessEnv)) {
        Fail "TAURI_SIGNING_PRIVATE_KEY_PASSWORD was not cleared after success"
    }
    if (Test-V4UpdaterProductionCredential -Target $testTarget) {
        Fail "Session credential was not deleted after success"
    }

    # Case B: Failure path
    [SkyAutoPlayer.V4UpdaterCredentialBroker]::WriteSessionCredential($testTarget, $testSentinel)
    $failureObserved = $false
    try {
        $env:SKY_MOCK_ORCHESTRATOR_FAIL = '1'
        Invoke-TestBuildCandidateBroker -Target $testTarget -Action {
            & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
                -File $mockOrchestratorPath `
                -ExpectedSourceSha "0123456789abcdef0123456789abcdef01234567" `
                -Version "4.0.0-rc.1" `
                -Channel "beta" `
                -UpdaterPrivateKeyPath "C:\path\to\key.key"
            if ($LASTEXITCODE -ne 0) { throw "Mock orchestrator failed intentionally" }
        }
    } catch {
        $failureObserved = $true
    } finally {
        Remove-Item Env:SKY_MOCK_ORCHESTRATOR_FAIL -ErrorAction SilentlyContinue
    }
    if (-not $failureObserved) {
        Fail "Expected failure was not thrown"
    }
    $postFailProcessEnv = [Environment]::GetEnvironmentVariable("TAURI_SIGNING_PRIVATE_KEY_PASSWORD", "Process")
    if (-not [string]::IsNullOrEmpty($postFailProcessEnv)) {
        Fail "TAURI_SIGNING_PRIVATE_KEY_PASSWORD was not cleared after failure"
    }
    if (Test-V4UpdaterProductionCredential -Target $testTarget) {
        Fail "Session credential was not deleted after failure"
    }

    # Case C: Ambient environment rejection in v4_release_pipeline.ps1
    $savedKeyPathEnv = $env:TAURI_SIGNING_PRIVATE_KEY_PATH
    $savedPassEnv = $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD
    $dummyKeyPath = Join-Path $fixtureRoot "test.key"
    New-Item -ItemType File -Path $dummyKeyPath -Force | Out-Null
    try {
        $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = "ambient-forbidden-value"
        $pipelineOutput = & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
            -File (Join-Path $PSScriptRoot "v4_release_pipeline.ps1") `
            -State BuildCandidate `
            -Version "4.0.0-rc.1" `
            -Channel "beta" `
            -Tag "v4.0.0-rc.1" `
            -SourceSha "02d3a973c23ca17ff331563e4f49fe090d3be207" `
            -WorkflowSha "02d3a973c23ca17ff331563e4f49fe090d3be207" `
            -StateRoot (Join-Path ([IO.Path]::GetTempPath()) ("state-" + [guid]::NewGuid().ToString("N"))) `
            -UpdaterPrivateKeyPath $dummyKeyPath 2>&1 | Out-String
        if ($pipelineOutput -notmatch "ambient updater key or password environment is forbidden") {
            Fail "v4_release_pipeline did not reject ambient TAURI_SIGNING_PRIVATE_KEY_PASSWORD: $pipelineOutput"
        }
    } finally {
        if ($null -ne $savedPassEnv) {
            $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = $savedPassEnv
        } else {
            Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD -ErrorAction SilentlyContinue
        }
    }

    Write-Host "Test 7: PASS"

    Write-Host "================================================================="
    Write-Host " [PASS] All V4 updater credential broker tests passed"
    Write-Host "================================================================="
} finally {
    Remove-V4UpdaterProductionCredential -Target $testTarget
    if ($null -ne $fixtureRoot -and (Test-Path -LiteralPath $fixtureRoot)) {
        Remove-Item -LiteralPath $fixtureRoot -Recurse -Force -ErrorAction SilentlyContinue
    }
}
