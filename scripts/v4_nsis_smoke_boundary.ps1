# scripts/v4_nsis_smoke_boundary.ps1
# Bounded Windows adapter for hermetic production-identity NSIS smoke testing.
# Isolates registry state, profile/application data, shortcuts, and filesystem roots.
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Get-V4NsisMonitoredRegistryKeys {
    <#
    .SYNOPSIS
    Returns the list of per-user product and uninstall registry keys that
    current and historical Sky Auto Player NSIS installers or harnesses interact with.
    #>
    return @(
        [ordered]@{ Key = 'HKCU:\Software\github\Sky Auto Player'; Parent = 'HKCU:\Software\github' },
        [ordered]@{ Key = 'HKCU:\Software\pumni\Sky Auto Player'; Parent = 'HKCU:\Software\pumni' },
        [ordered]@{ Key = 'HKCU:\Software\Sky Auto Player Team\Sky Auto Player'; Parent = 'HKCU:\Software\Sky Auto Player Team' },
        [ordered]@{ Key = 'HKCU:\Software\Sky Auto Player'; Parent = 'HKCU:\Software' },
        [ordered]@{ Key = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Sky Auto Player'; Parent = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall' },
        [ordered]@{ Key = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\io.github.pumni.skyautoplayer'; Parent = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall' }
    )
}

function Protect-V4NsisRegistryState {
    <#
    .SYNOPSIS
    Captures an exact snapshot of monitored product and uninstall registry keys.
    #>
    [CmdletBinding()]
    param(
        [System.Collections.IList]$Targets = $null
    )

    $effectiveTargets = if ($null -eq $Targets) { Get-V4NsisMonitoredRegistryKeys } else { $Targets }
    $snapshots = [ordered]@{}

    foreach ($target in $effectiveTargets) {
        $keyPath = $target.Key
        $parentPath = $target.Parent

        $parentExisted = Test-Path -LiteralPath $parentPath
        $keyExisted = Test-Path -LiteralPath $keyPath

        $properties = [ordered]@{}
        $subKeyNames = @()

        if ($keyExisted) {
            $regItem = Get-Item -LiteralPath $keyPath
            foreach ($propName in $regItem.GetValueNames()) {
                $properties[$propName] = [ordered]@{
                    Value = $regItem.GetValue($propName, $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
                    Kind  = $regItem.GetValueKind($propName)
                }
            }
            $subKeyNames = @($regItem.GetSubKeyNames())
        }

        $snapshots[$keyPath] = [ordered]@{
            KeyPath        = $keyPath
            ParentPath     = $parentPath
            ParentExisted  = $parentExisted
            KeyExisted     = $keyExisted
            Properties     = $properties
            SubKeyNames    = $subKeyNames
        }
    }

    return $snapshots
}

function Test-V4RegistryValueEqual {
    param($Value1, $Value2, [Microsoft.Win32.RegistryValueKind]$Kind)

    if ($null -eq $Value1 -and $null -eq $Value2) { return $true }
    if ($null -eq $Value1 -or $null -eq $Value2) { return $false }

    if ($Kind -eq [Microsoft.Win32.RegistryValueKind]::Binary) {
        $b1 = [byte[]]$Value1
        $b2 = [byte[]]$Value2
        if ($b1.Length -ne $b2.Length) { return $false }
        for ($i = 0; $i -lt $b1.Length; $i++) {
            if ($b1[$i] -ne $b2[$i]) { return $false }
        }
        return $true
    }

    if ($Kind -eq [Microsoft.Win32.RegistryValueKind]::MultiString) {
        $s1 = [string[]]$Value1
        $s2 = [string[]]$Value2
        if ($s1.Length -ne $s2.Length) { return $false }
        for ($i = 0; $i -lt $s1.Length; $i++) {
            if ($s1[$i] -ne $s2[$i]) { return $false }
        }
        return $true
    }

    return ($Value1 -eq $Value2)
}

function Assert-V4NsisRegistryEquivalence {
    <#
    .SYNOPSIS
    Asserts that the monitored registry keys match the snapshot state exactly.
    Fails closed if any unexpected key, value, kind, or subkey residue exists.
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [System.Collections.IDictionary]$Snapshots
    )

    foreach ($keyPath in $Snapshots.Keys) {
        $snapshot = $Snapshots[$keyPath]
        $keyExisted = $snapshot.KeyExisted

        if (-not $keyExisted) {
            if (Test-Path -LiteralPath $keyPath) {
                throw "Registry residue detected: key '$keyPath' was absent prior to smoke test but still exists after restoration."
            }
        } else {
            if (-not (Test-Path -LiteralPath $keyPath)) {
                throw "Registry restoration failed: key '$keyPath' existed prior to smoke test but is missing after restoration."
            }

            $regItem = Get-Item -LiteralPath $keyPath
            $currentNames = @($regItem.GetValueNames())
            $snapshotProps = $snapshot.Properties

            if ($currentNames.Count -ne $snapshotProps.Count) {
                throw "Registry residue detected in '$keyPath': expected $($snapshotProps.Count) values, found $($currentNames.Count)."
            }

            foreach ($name in $currentNames) {
                if (-not $snapshotProps.Contains($name)) {
                    $displayName = if ($name -eq '') { '(Default)' } else { "'$name'" }
                    throw "Registry residue detected in '$keyPath': unexpected value $displayName found after restoration."
                }
            }

            foreach ($name in $snapshotProps.Keys) {
                $expected = $snapshotProps[$name]
                $actualKind = $regItem.GetValueKind($name)
                $expectedKind = [Microsoft.Win32.RegistryValueKind]$expected.Kind

                if ($actualKind -ne $expectedKind) {
                    $displayName = if ($name -eq '') { '(Default)' } else { "'$name'" }
                    throw "Registry restoration kind mismatch in '$keyPath' for value ${displayName}: expected $expectedKind, got $actualKind."
                }

                $actualValue = $regItem.GetValue($name, $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
                $expectedValue = $expected.Value

                $valuesEqual = Test-V4RegistryValueEqual -Value1 $actualValue -Value2 $expectedValue -Kind $actualKind
                if (-not $valuesEqual) {
                    $displayName = if ($name -eq '') { '(Default)' } else { "'$name'" }
                    throw "Registry restoration content mismatch in '$keyPath' for value ${displayName}: expected '$expectedValue', got '$actualValue'."
                }
            }

            $currentSubKeys = @($regItem.GetSubKeyNames())
            $expectedSubKeys = @($snapshot.SubKeyNames)
            if ($currentSubKeys.Count -ne $expectedSubKeys.Count) {
                throw "Registry residue detected in '$keyPath': expected $($expectedSubKeys.Count) subkeys, found $($currentSubKeys.Count)."
            }
            foreach ($subName in $currentSubKeys) {
                if ($expectedSubKeys -notcontains $subName) {
                    throw "Registry residue detected in '$keyPath': unexpected subkey '$subName' found after restoration."
                }
            }
        }
    }
}

function Restore-V4NsisRegistryState {
    <#
    .SYNOPSIS
    Restores the monitored registry keys to the exact state recorded in the snapshot.
    Enforces exact value kinds (including default values) and final equivalence assertion.
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [System.Collections.IDictionary]$Snapshots
    )

    foreach ($keyPath in $Snapshots.Keys) {
        $snapshot = $Snapshots[$keyPath]
        $parentPath = $snapshot.ParentPath
        $keyExisted = $snapshot.KeyExisted
        $parentExisted = $snapshot.ParentExisted

        if (-not $keyExisted) {
            # Key was absent before test: remove completely if created during test
            if (Test-Path -LiteralPath $keyPath) {
                Remove-Item -LiteralPath $keyPath -Recurse -Force -ErrorAction Stop
            }

            # If parent manufacturer key did not exist before and is now empty, remove parent
            if (-not $parentExisted -and ($parentPath -ne 'HKCU:\Software') -and (Test-Path -LiteralPath $parentPath)) {
                $parentItem = Get-Item -LiteralPath $parentPath -ErrorAction SilentlyContinue
                if ($null -ne $parentItem -and $parentItem.SubKeyCount -eq 0 -and $parentItem.ValueCount -eq 0) {
                    Remove-Item -LiteralPath $parentPath -Force -ErrorAction Stop
                }
            }
        } else {
            # Key was present before test: ensure it exists and has exact properties
            if (-not (Test-Path -LiteralPath $keyPath)) {
                New-Item -Path $keyPath -Force | Out-Null
            }

            $subPath = $keyPath.Substring('HKCU:\'.Length)
            $writable = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($subPath, $true)
            if ($null -eq $writable) {
                throw "Failed to open registry key '$keyPath' for writable restoration."
            }

            try {
                $regItem = Get-Item -LiteralPath $keyPath
                $currentNames = @($regItem.GetValueNames())
                $snapshotProps = $snapshot.Properties

                # Remove properties that were added during test
                foreach ($currentName in $currentNames) {
                    if (-not $snapshotProps.Contains($currentName)) {
                        $writable.DeleteValue($currentName, $false)
                    }
                }

                # Remove subkeys that were added during test
                $currentSubKeys = @($regItem.GetSubKeyNames())
                $snapshotSubKeys = @($snapshot.SubKeyNames)
                foreach ($subName in $currentSubKeys) {
                    if ($snapshotSubKeys -notcontains $subName) {
                        $writable.DeleteSubKeyTree($subName, $false)
                    }
                }

                # Restore original snapshot properties with exact RegistryValueKind
                foreach ($propName in $snapshotProps.Keys) {
                    $propRecord = $snapshotProps[$propName]
                    $propValue = $propRecord.Value
                    $propKind = [Microsoft.Win32.RegistryValueKind]$propRecord.Kind

                    $writable.SetValue($propName, $propValue, $propKind)
                }
            } finally {
                $writable.Close()
            }
        }
    }

    # Final equivalence assertion: fail closed if any monitored residue remains
    Assert-V4NsisRegistryEquivalence -Snapshots $Snapshots
}

function Remove-V4DirectoryWithRetry {
    <#
    .SYNOPSIS
    Removes a directory with bounded retries and fails closed if residue remains.
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path,
        [int]$MaxAttempts = 10,
        [int]$DelayMilliseconds = 200
    )

    if (-not (Test-Path -LiteralPath $Path)) {
        return
    }

    for ($attempt = 1; $attempt -le $MaxAttempts; $attempt++) {
        try {
            Remove-Item -LiteralPath $Path -Recurse -Force -ErrorAction Stop
        } catch {
            # Bounded retry on lock contention
        }

        if (-not (Test-Path -LiteralPath $Path)) {
            return
        }
        Start-Sleep -Milliseconds $DelayMilliseconds
    }

    if (Test-Path -LiteralPath $Path) {
        throw "Failed to clean up test directory after $MaxAttempts attempts: residue remains at '$Path'"
    }
}

function Stop-V4TrackedProcesses {
    <#
    .SYNOPSIS
    Stops any tracked child processes and waits for exit.
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [System.Collections.IList]$Processes
    )

    foreach ($proc in $Processes) {
        if ($null -ne $proc) {
            try {
                if (-not $proc.HasExited) {
                    Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
                    $proc.WaitForExit(5000)
                }
            } catch {
                # Process may have exited concurrently
            }
        }
    }
}

function Enter-V4NsisSmokeScope {
    <#
    .SYNOPSIS
    Enters an isolated NSIS smoke test scope. Captures registry state and sets up
    throwaway application data isolation.
    #>
    [CmdletBinding()]
    param(
        [string]$InstallRoot = $null,
        [string]$AppDataRoot = $null,
        [switch]$ManageInstallRootCleanup = $true,
        [switch]$ManageAppDataCleanup = $true,
        [System.Collections.IList]$RegistryTargets = $null
    )

    $snapshots = Protect-V4NsisRegistryState -Targets $RegistryTargets
    $previousAppDataRoot = [Environment]::GetEnvironmentVariable('SKY_APP_DATA_ROOT', 'Process')

    $createdAppData = $false
    $resolvedAppDataRoot = $AppDataRoot
    if ([string]::IsNullOrWhiteSpace($resolvedAppDataRoot)) {
        $resolvedAppDataRoot = Join-Path ([IO.Path]::GetTempPath()) ("sky-v4-smoke-appdata-" + [guid]::NewGuid().ToString("N"))
        $createdAppData = $true
    }

    New-Item -ItemType Directory -Path $resolvedAppDataRoot -Force | Out-Null
    [Environment]::SetEnvironmentVariable('SKY_APP_DATA_ROOT', $resolvedAppDataRoot, 'Process')

    $initialErrorCount = if ($null -ne $global:Error) { $global:Error.Count } else { 0 }

    return [PSCustomObject]@{
        RegistrySnapshots        = $snapshots
        PreviousAppDataRoot      = $previousAppDataRoot
        AppDataRoot              = $resolvedAppDataRoot
        CreatedAppData           = $createdAppData
        ManageAppDataCleanup     = [bool]$ManageAppDataCleanup
        InstallRoot              = $InstallRoot
        ManageInstallRootCleanup = [bool]$ManageInstallRootCleanup
        TrackedProcesses         = [System.Collections.Generic.List[System.Diagnostics.Process]]::new()
        InitialErrorCount        = $initialErrorCount
    }
}

function Exit-V4NsisSmokeScope {
    <#
    .SYNOPSIS
    Exits the isolated NSIS smoke test scope. Guarantees process termination,
    registry restoration, environment variable cleanup, and fail-closed directory cleanup.
    Performs attempt-all cleanup across all stages before raising aggregate errors.
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [PSCustomObject]$Scope,
        [System.Exception]$OriginalError = $null
    )

    $cleanupErrors = [System.Collections.Generic.List[string]]::new()

    # 1. Terminate any running smoke processes
    try {
        Stop-V4TrackedProcesses -Processes $Scope.TrackedProcesses
    } catch {
        $cleanupErrors.Add("Failed to stop tracked processes: $($_.Exception.Message)")
    }

    # 2. Restore registry state and assert equivalence
    try {
        Restore-V4NsisRegistryState -Snapshots $Scope.RegistrySnapshots
    } catch {
        $cleanupErrors.Add("Failed to restore registry state: $($_.Exception.Message)")
    }

    # 3. Restore SKY_APP_DATA_ROOT environment variable
    try {
        if ($null -eq $Scope.PreviousAppDataRoot) {
            Remove-Item Env:SKY_APP_DATA_ROOT -ErrorAction SilentlyContinue
        } else {
            [Environment]::SetEnvironmentVariable('SKY_APP_DATA_ROOT', $Scope.PreviousAppDataRoot, 'Process')
        }
    } catch {
        $cleanupErrors.Add("Failed to restore SKY_APP_DATA_ROOT: $($_.Exception.Message)")
    }

    # 4. Clean up throwaway AppData root
    try {
        if ($Scope.ManageAppDataCleanup -and -not [string]::IsNullOrWhiteSpace($Scope.AppDataRoot) -and (Test-Path -LiteralPath $Scope.AppDataRoot)) {
            Remove-V4DirectoryWithRetry -Path $Scope.AppDataRoot
        }
    } catch {
        $cleanupErrors.Add("Failed to clean up AppData root '$($Scope.AppDataRoot)': $($_.Exception.Message)")
    }

    # 5. Clean up InstallRoot if requested
    try {
        if ($Scope.ManageInstallRootCleanup -and -not [string]::IsNullOrWhiteSpace($Scope.InstallRoot) -and (Test-Path -LiteralPath $Scope.InstallRoot)) {
            Remove-V4DirectoryWithRetry -Path $Scope.InstallRoot
        }
    } catch {
        $cleanupErrors.Add("Failed to clean up InstallRoot '$($Scope.InstallRoot)': $($_.Exception.Message)")
    }

    if ($cleanupErrors.Count -gt 0) {
        $message = "Exit-V4NsisSmokeScope failed with $($cleanupErrors.Count) cleanup error(s):`n - " + ($cleanupErrors -join "`n - ")
        if ($null -ne $OriginalError) {
            $message += "`n`nOriginal error before cleanup: $($OriginalError.Message)"
        } elseif ($null -ne $global:Error -and $global:Error.Count -gt $Scope.InitialErrorCount) {
            $prior = $global:Error[0].Exception.Message
            $message += "`n`nPrior context error: $prior"
        }
        throw $message
    }
}

function Invoke-V4NsisInstaller {
    <#
    .SYNOPSIS
    Invokes the production-identity NSIS installer in silent, no-shortcut mode
    targeting an explicit install root.
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)] [string]$InstallerPath,
        [Parameter(Mandatory = $true)] [string]$InstallRoot,
        [string[]]$AdditionalArguments = @()
    )

    if (-not (Test-Path -LiteralPath $InstallerPath)) {
        throw "Installer executable does not exist: $InstallerPath"
    }

    # In NSIS, /D=<path> must be the final parameter on the command line without quotes
    $argumentList = @('/S', '/NS') + $AdditionalArguments + @("/D=$InstallRoot")
    $proc = Start-Process -FilePath $InstallerPath -ArgumentList $argumentList -WindowStyle Hidden -Wait -PassThru
    if ($proc.ExitCode -ne 0) {
        throw "Tauri NSIS installer exited with non-zero code $($proc.ExitCode)"
    }
    return $proc
}

function Invoke-V4NsisUninstaller {
    <#
    .SYNOPSIS
    Invokes the NSIS uninstaller silently.
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)] [string]$UninstallerPath,
        [string[]]$AdditionalArguments = @()
    )

    if (-not (Test-Path -LiteralPath $UninstallerPath)) {
        throw "Uninstaller executable does not exist: $UninstallerPath"
    }

    $argumentList = @('/S') + $AdditionalArguments
    $proc = Start-Process -FilePath $UninstallerPath -ArgumentList $argumentList -WindowStyle Hidden -Wait -PassThru
    if ($proc.ExitCode -ne 0) {
        throw "Tauri NSIS uninstaller exited with non-zero code $($proc.ExitCode)"
    }
    return $proc
}
