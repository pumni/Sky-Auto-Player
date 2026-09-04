$ErrorActionPreference = "Stop"
$fixtureRoot = Join-Path ([IO.Path]::GetTempPath()) ("sky-v4-updater-rotation-" + [guid]::NewGuid().ToString("N"))
$oldKeyPath = Join-Path $fixtureRoot "old.key"
$newKeyPath = Join-Path $fixtureRoot "new.key"
$payloadPath = Join-Path $fixtureRoot "rotation-payload.bin"
$oldSignaturePath = Join-Path $fixtureRoot "old.sig"
$newSignaturePath = Join-Path $fixtureRoot "new.sig"
New-Item -ItemType Directory -Path $fixtureRoot -Force | Out-Null
try {
    [IO.File]::WriteAllBytes($payloadPath, [Text.Encoding]::UTF8.GetBytes("Sky Auto Player v4 updater rotation fixture"))
    Push-Location (Join-Path $PSScriptRoot "..\desktop")
    try {
        bun run tauri signer generate --ci --password "" --force -w $oldKeyPath | Out-Null
        bun run tauri signer generate --ci --password "" --force -w $newKeyPath | Out-Null
        bun run tauri signer sign --private-key-path $oldKeyPath --password "" $payloadPath | Out-Null
        Move-Item -LiteralPath "$payloadPath.sig" -Destination $oldSignaturePath -Force
        bun run tauri signer sign --private-key-path $newKeyPath --password "" $payloadPath | Out-Null
        Move-Item -LiteralPath "$payloadPath.sig" -Destination $newSignaturePath -Force
    } finally {
        Pop-Location
    }
    cargo xtask updater-trust rotation-self-test `
        --old-public "$oldKeyPath.pub" `
        --new-public "$newKeyPath.pub" `
        --old-signature $oldSignaturePath `
        --new-signature $newSignaturePath `
        --payload $payloadPath
} finally {
    if ((Resolve-Path -LiteralPath $fixtureRoot -ErrorAction SilentlyContinue) -and
        ([IO.Path]::GetFullPath($fixtureRoot).StartsWith([IO.Path]::GetFullPath([IO.Path]::GetTempPath()), [StringComparison]::OrdinalIgnoreCase))) {
        Remove-Item -LiteralPath $fixtureRoot -Recurse -Force -ErrorAction SilentlyContinue
    }
}
