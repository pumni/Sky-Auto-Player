$ErrorActionPreference = 'Stop'

$repoRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$script = Join-Path $PSScriptRoot 'prepare_chord_width_pilot_v1.ps1'

function Assert-FailsClosed {
    param(
        [string]$Label,
        [string[]]$Arguments,
        [string]$ExpectedPattern
    )
    $failed = $false
    $message = ''
    try {
        & pwsh -NoProfile -File $script @Arguments 2>&1 | Out-Null
        if ($LASTEXITCODE -ne 0) {
            $failed = $true
        }
    } catch {
        $failed = $true
        $message = $_.Exception.Message
    }
    if (-not $failed) {
        # Capture stderr explicitly when pwsh exits nonzero without throwing
        $output = (& pwsh -NoProfile -File $script @Arguments 2>&1 | Out-String)
        if ($LASTEXITCODE -eq 0) {
            throw "Expected fail-closed for [$Label], but command succeeded."
        }
        $message = $output
    } else {
        $output = (& pwsh -NoProfile -File $script @Arguments 2>&1 | Out-String)
        $message = $output
    }
    if ($ExpectedPattern -and ($message -notmatch $ExpectedPattern)) {
        throw "Fail-closed message mismatch for [$Label]. Expected /$ExpectedPattern/, got: $message"
    }
    Write-Output "PASS fail-closed: $Label"
}

# 1. Owner + PlanOnly / PreflightOnly contradiction must fail closed
Assert-FailsClosed -Label 'owner+planonly contradiction' -Arguments @('-Mode', 'PlanOnly', '-OwnerApprovedReceiveOnlyRun', '-AllowDirtyForContractTest') -ExpectedPattern 'contradiction'
Assert-FailsClosed -Label 'owner+preflight contradiction' -Arguments @('-Mode', 'PreflightOnly', '-OwnerApprovedReceiveOnlyRun', '-AllowDirtyForContractTest') -ExpectedPattern 'contradiction'

# 2. Missing owner approval on Execute must fail closed
Assert-FailsClosed -Label 'execute without owner approval' -Arguments @('-Mode', 'Execute', '-AllowDirtyForContractTest') -ExpectedPattern 'OwnerApprovedReceiveOnlyRun'

# 3. Execute even WITH owner flag remains LOCKED in R2-P
Assert-FailsClosed -Label 'execute locked in R2-P' -Arguments @('-Mode', 'Execute', '-OwnerApprovedReceiveOnlyRun', '-AllowDirtyForContractTest') -ExpectedPattern 'LOCKED'

# 4. Wrong receiver role or game_target=true must fail closed
Assert-FailsClosed -Label 'wrong receiver role' -Arguments @('-Mode', 'PlanOnly', '-AuthorizedReceiverRole', 'InertFocusProbe', '-AllowDirtyForContractTest') -ExpectedPattern 'unauthorized receiver role'
Assert-FailsClosed -Label 'game_target true forbidden' -Arguments @('-Mode', 'PlanOnly', '-AuthorizedGameTarget', '-AllowDirtyForContractTest') -ExpectedPattern 'game_target=true'

# 5. Unsupported or duplicate widths must fail closed
Assert-FailsClosed -Label 'unsupported width 3' -Arguments @('-Mode', 'PlanOnly', '-Widths', '1,3,5', '-AllowDirtyForContractTest') -ExpectedPattern 'unsupported pilot chord width'
Assert-FailsClosed -Label 'duplicate width 5' -Arguments @('-Mode', 'PlanOnly', '-Widths', '5,5', '-AllowDirtyForContractTest') -ExpectedPattern 'duplicate pilot chord width'

# 6. OutputRoot inside repo checkout must fail closed
Assert-FailsClosed -Label 'output inside checkout' -Arguments @('-Mode', 'PlanOnly', '-OutputRoot', (Join-Path $repoRoot 'tmp-out'), '-AllowDirtyForContractTest') -ExpectedPattern 'outside the repository checkout'

# 7. PreflightOnly without manifest or build must fail closed
Assert-FailsClosed -Label 'preflight without manifest' -Arguments @('-Mode', 'PreflightOnly', '-AllowDirtyForContractTest') -ExpectedPattern 'requires either -BuildHarness or a verified -HarnessManifestPath'

# 8. Controlled fake-manifest tests (stale SHA and binary hash mismatch) — zero real receiver or SendInput
$tempRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("sky-pilot-preflight-test-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force -Path $tempRoot | Out-Null
try {
    $fakeBin = Join-Path $tempRoot 'fake-rt-native-acceptance.exe'
    'fake-nonphysical-binary-bytes' | Set-Content -LiteralPath $fakeBin -Encoding ascii
    $fakeBinHash = (Get-FileHash -LiteralPath $fakeBin -Algorithm SHA256).Hash.ToLowerInvariant()
    $headSha = (git -C $repoRoot rev-parse HEAD).Trim()
    $treeSha = (git -C $repoRoot rev-parse 'HEAD^{tree}').Trim()
    $lockSha = (Get-FileHash -LiteralPath (Join-Path $repoRoot 'rust/Cargo.lock') -Algorithm SHA256).Hash.ToLowerInvariant()

    # Stale SHA manifest fails closed
    $staleManifest = Join-Path $tempRoot 'stale-manifest.json'
    ([ordered]@{
        pilot_identity = 'ChordWidthPilotV1'
        source_revision = '0000000000000000000000000000000000000000'
        source_tree = $treeSha
        cargo_lock_sha256 = $lockSha
        binary_path = $fakeBin
        binary_sha256 = $fakeBinHash
    } | ConvertTo-Json) | Set-Content -LiteralPath $staleManifest -Encoding utf8NoBOM
    Assert-FailsClosed -Label 'stale manifest SHA' -Arguments @('-Mode', 'PreflightOnly', '-HarnessManifestPath', $staleManifest, '-AllowDirtyForContractTest') -ExpectedPattern 'stale manifest'

    # Tampered binary hash fails closed
    $badHashManifest = Join-Path $tempRoot 'bad-hash-manifest.json'
    ([ordered]@{
        pilot_identity = 'ChordWidthPilotV1'
        source_revision = $headSha
        source_tree = $treeSha
        cargo_lock_sha256 = $lockSha
        binary_path = $fakeBin
        binary_sha256 = 'ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff'
    } | ConvertTo-Json) | Set-Content -LiteralPath $badHashManifest -Encoding utf8NoBOM
    Assert-FailsClosed -Label 'binary hash mismatch' -Arguments @('-Mode', 'PreflightOnly', '-HarnessManifestPath', $badHashManifest, '-AllowDirtyForContractTest') -ExpectedPattern 'binary SHA-256 mismatch'

    # Valid controlled fake-manifest PreflightOnly succeeds with zero native invocation
    $validManifest = Join-Path $tempRoot 'valid-manifest.json'
    ([ordered]@{
        pilot_identity = 'ChordWidthPilotV1'
        source_revision = $headSha
        source_tree = $treeSha
        cargo_lock_sha256 = $lockSha
        binary_path = $fakeBin
        binary_sha256 = $fakeBinHash
    } | ConvertTo-Json) | Set-Content -LiteralPath $validManifest -Encoding utf8NoBOM

    $rawJson = & pwsh -NoProfile -File $script -Mode PreflightOnly -HarnessManifestPath $validManifest -OutputRoot $tempRoot -AllowDirtyForContractTest
    if ($LASTEXITCODE -ne 0) { throw "Valid fake-manifest PreflightOnly failed with exit code $LASTEXITCODE" }
    $plan = $rawJson | ConvertFrom-Json
    if ($plan.pilot_identity -ne 'ChordWidthPilotV1') { throw 'unexpected pilot_identity' }
    if ($plan.receiver_started -ne $false -or $plan.harness_started -ne $false -or $plan.no_native_invocation -ne $true -or $plan.sendinput_calls -ne 0) {
        throw 'PreflightOnly must report zero native invocation'
    }
    if ($plan.planned_smoke_session_count -ne 3) { throw 'expected 3 quiet smoke sessions' }
    if ($plan.receiver_contract.role -ne 'ReceiveOnly' -or $plan.receiver_contract.game_target -ne $false) {
        throw 'receiver_contract mismatch'
    }
    Write-Output "PASS controlled fake-manifest PreflightOnly (3 sessions, zero native invocation)"
} finally {
    Remove-Item -Recurse -Force $tempRoot -ErrorAction SilentlyContinue
}
Write-Output 'ALL ChordWidthPilotV1 preflight contract tests PASSED.'
