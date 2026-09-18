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
    param()

    $targets = Get-V4NsisMonitoredRegistryKeys
    $snapshots = [ordered]@{}

    foreach ($target in $targets) {
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
                    Value = $regItem.GetValue($propName)
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

function Restore-V4NsisRegistryState {
    <#
    .SYNOPSIS
    Restores the monitored registry keys to the exact state recorded in the snapshot.
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
                    Remove-Item -LiteralPath $parentPath -Force -ErrorAction SilentlyContinue
                }
            }
        } else {
            # Key was present before test: ensure it exists and has exact properties
            if (-not (Test-Path -LiteralPath $keyPath)) {
                New-Item -Path $keyPath -Force | Out-Null
            }

            $regItem = Get-Item -LiteralPath $keyPath
            $currentNames = @($regItem.GetValueNames())
            $snapshotProps = $snapshot.Properties

            # Remove properties that were added during test
            foreach ($currentName in $currentNames) {
                if (-not $snapshotProps.Contains($currentName)) {
                    if ($currentName -eq '') {
                        # Default value: delete using .NET RegistryKey to remove the value entry
                        $subPath = $keyPath.Substring('HKCU:\'.Length)
                        $writable = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($subPath, $true)
                        if ($null -ne $writable) {
                            try {
                                $writable.DeleteValue('', $false)
                            } finally {
                                $writable.Close()
                            }
                        }
                    } else {
                        Remove-ItemProperty -LiteralPath $keyPath -Name $currentName -Force -ErrorAction SilentlyContinue
                    }
                }
            }

            # Remove subkeys that were added during test
            $currentSubKeys = @($regItem.GetSubKeyNames())
            $snapshotSubKeys = @($snapshot.SubKeyNames)
            foreach ($subName in $currentSubKeys) {
                if ($snapshotSubKeys -notcontains $subName) {
                    $subPath = Join-Path $keyPath $subName
                    Remove-Item -LiteralPath $subPath -Recurse -Force -ErrorAction SilentlyContinue
                }
            }

            # Restore original snapshot properties
            foreach ($propName in $snapshotProps.Keys) {
                $propRecord = $snapshotProps[$propName]
                $propValue = $propRecord.Value
                $propKind = $propRecord.Kind

                if ($propName -eq '') {
                    Set-Item -LiteralPath $keyPath -Value $propValue -Force
                } else {
                    Set-ItemProperty -LiteralPath $keyPath -Name $propName -Value $propValue -Type $propKind -Force
                }
            }
        }
    }
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
        [switch]$ManageInstallRootCleanup = $true
    )

    $snapshots = Protect-V4NsisRegistryState
    $previousAppDataRoot = [Environment]::GetEnvironmentVariable('SKY_APP_DATA_ROOT', 'Process')

    $createdAppData = $false
    $resolvedAppDataRoot = $AppDataRoot
    if ([string]::IsNullOrWhiteSpace($resolvedAppDataRoot)) {
        $resolvedAppDataRoot = Join-Path ([IO.Path]::GetTempPath()) ("sky-v4-smoke-appdata-" + [guid]::NewGuid().ToString("N"))
        $createdAppData = $true
    }

    New-Item -ItemType Directory -Path $resolvedAppDataRoot -Force | Out-Null
    [Environment]::SetEnvironmentVariable('SKY_APP_DATA_ROOT', $resolvedAppDataRoot, 'Process')

    return [PSCustomObject]@{
        RegistrySnapshots        = $snapshots
        PreviousAppDataRoot      = $previousAppDataRoot
        AppDataRoot              = $resolvedAppDataRoot
        CreatedAppData           = $createdAppData
        InstallRoot              = $InstallRoot
        ManageInstallRootCleanup = $ManageInstallRootCleanup.IsPresent
        TrackedProcesses         = [System.Collections.Generic.List[System.Diagnostics.Process]]::new()
    }
}

function Exit-V4NsisSmokeScope {
    <#
    .SYNOPSIS
    Exits the isolated NSIS smoke test scope. Guarantees process termination,
    registry restoration, environment variable cleanup, and fail-closed directory cleanup.
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [PSCustomObject]$Scope
    )

    # 1. Terminate any running smoke processes
    Stop-V4TrackedProcesses -Processes $Scope.TrackedProcesses

    # 2. Restore registry state
    Restore-V4NsisRegistryState -Snapshots $Scope.RegistrySnapshots

    # 3. Restore SKY_APP_DATA_ROOT environment variable
    if ($null -eq $Scope.PreviousAppDataRoot) {
        Remove-Item Env:SKY_APP_DATA_ROOT -ErrorAction SilentlyContinue
    } else {
        [Environment]::SetEnvironmentVariable('SKY_APP_DATA_ROOT', $Scope.PreviousAppDataRoot, 'Process')
    }

    # 4. Clean up throwaway AppData root
    if ($Scope.CreatedAppData -and (Test-Path -LiteralPath $Scope.AppDataRoot)) {
        Remove-V4DirectoryWithRetry -Path $Scope.AppDataRoot
    }

    # 5. Clean up InstallRoot if requested
    if ($Scope.ManageInstallRootCleanup -and -not [string]::IsNullOrWhiteSpace($Scope.InstallRoot) -and (Test-Path -LiteralPath $Scope.InstallRoot)) {
        Remove-V4DirectoryWithRetry -Path $Scope.InstallRoot
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
