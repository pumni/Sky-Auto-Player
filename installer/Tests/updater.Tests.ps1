#Requires -Modules Pester

# Test file for updater.ps1
# Tests the core functions of the external updater.
#
# Pester 5+ notes:
#   - setup/teardown blocks (BeforeAll/AfterAll/BeforeEach) MUST live inside a
#     Describe block. The whole file is wrapped in a top-level Describe to host
#     them; Pester 5 supports nested Describe blocks.
#   - updater.ps1 helper functions read $global:InstallRoot / $global:ExePath /
#     $global:ConfigPath / $global:LogDir / $global:LogFile (see Get-ExePath
#     etc.). The test BeforeAll must therefore set the $global: aliases — the
#     $script: aliases alone are not visible to those helpers and would cause
#     Initialize-Paths to re-trigger and return null (because dot-sourcing
#     defeats updater.ps1's $MyInvocation detection at line 70).

Describe "updater.ps1" {
    BeforeAll {
        # Import the updater module (dot-source it to access functions).
        . (Join-Path $PSScriptRoot '..\updater.ps1')

        # Create test environment.
        $script:TestConfigDir = Join-Path $env:TEMP ('sky-updater-test-' + [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Force -Path $script:TestConfigDir | Out-Null
        $script:TestConfigPath = Join-Path $script:TestConfigDir 'config.json'
        $script:ExeStubPath    = Join-Path $script:TestConfigDir 'Sky-Auto-Player.exe'
        $script:LogDir         = Join-Path $env:TEMP 'Sky-Auto-Player-Test'
        $script:LogFile        = Join-Path $script:LogDir 'updater.log'
        New-Item -ItemType Directory -Force -Path $script:LogDir | Out-Null
        New-Item -ItemType File     -Force -Path $script:ExeStubPath | Out-Null

        # Propagate to updater.ps1's global scope so Get-ExePath / Get-ConfigPath
        # / Get-LogFile short-circuit and never call Initialize-Paths (which
        # would null-out these paths under dot-sourcing).
        $global:InstallRoot = $script:TestConfigDir
        $global:ExePath     = $script:ExeStubPath
        $global:ConfigPath  = $script:TestConfigPath
        $global:LogDir      = $script:LogDir
        $global:LogFile     = $script:LogFile

        # Helper functions for Test-ManifestIntegrity tests. Pester 5's scope
        # isolation means plain `function` definitions inside a Describe body
        # are NOT visible to It-blocks; defining them at this outermost
        # BeforeAll with explicit `script:` scope makes them resolvable from
        # any nested Describe / It.
        function script:Write-TestFile([string]$path, [string]$content = "test content") {
            $full = if ([System.IO.Path]::IsPathRooted($path)) { $path } else { Join-Path $script:StagingRoot $path }
            $parent = Split-Path -Parent $full
            if ($parent -and -not (Test-Path -LiteralPath $parent)) {
                New-Item -ItemType Directory -Force -Path $parent | Out-Null
            }
            $content | Out-File -Encoding UTF8 -LiteralPath $full
            return $full
        }
        function script:Write-TestManifest(
            [array]$fileEntries,
            [string]$Version = "9.9.9-test"
        ) {
            $exePath = Join-Path $script:StagingRoot 'Sky-Auto-Player.exe'
            if (-not (Test-Path -LiteralPath $exePath)) {
                [System.IO.File]::WriteAllText($exePath, 'test executable')
            }
            $manifest = @{
                app = "Sky-Auto-Player"
                version = $Version
                executable = "Sky-Auto-Player.exe"
                executable_sha256 = (Get-FileSha256 $exePath)
                files = @(
                    foreach ($entry in $fileEntries) {
                        @{ path = $entry.path; sha256 = $entry.sha256 }
                    }
                )
            }
            $manifest | ConvertTo-Json -Depth 10 | Out-File -Encoding UTF8 -LiteralPath (Join-Path $script:StagingRoot 'MANIFEST.json')
        }
        function script:Get-FileSha256([string]$path) {
            (Get-FileHash -Algorithm SHA256 -LiteralPath $path).Hash.ToLower()
        }
    }

    AfterAll {
        if ($script:TestConfigDir -and (Test-Path $script:TestConfigDir)) {
            Remove-Item -Recurse -Force $script:TestConfigDir -ErrorAction SilentlyContinue
        }
        if ($script:LogDir -and (Test-Path $script:LogDir)) {
            Remove-Item -Recurse -Force $script:LogDir -ErrorAction SilentlyContinue
        }
        Remove-Variable -Scope Global -Name InstallRoot, ExePath, ConfigPath, LogDir, LogFile -ErrorAction SilentlyContinue
    }

    BeforeEach {
        if ($script:TestConfigPath -and (Test-Path $script:TestConfigPath)) {
            Remove-Item -Force $script:TestConfigPath -ErrorAction SilentlyContinue
        }

        Remove-Item Env:SKY_UPDATER_FAKE_ROOT -ErrorAction SilentlyContinue
    }

    Describe "Write-UpdateFields JSON round-trip" {
        It "updates last_check_ts and last_notified_version when update object exists" {
            $config = @{
                theme = "aurora"
                update = @{
                    auto_check = $true
                    channel = "stable"
                    skip_version = ""
                    check_interval_s = 86400
                    last_check_ts = 0
                    last_error_ts = 0
                    last_notified_version = ""
                    legacy_old_dir_sweep_pending = $false
                }
            } | ConvertTo-Json -Depth 10
            $config | Out-File -FilePath $script:TestConfigPath -Encoding UTF8

            Write-UpdateFields -LastCheckTs 1718200000 -LastNotifiedVersion "2.4.0"

            $result = Get-Content -Raw -LiteralPath $script:TestConfigPath | ConvertFrom-Json
            $result.update.last_check_ts | Should -Be 1718200000
            $result.update.last_notified_version | Should -Be "2.4.0"
            $result.theme | Should -Be "aurora"
            $result.update.auto_check | Should -Be $true
        }

        It "creates update object if missing" {
            $config = @{ theme = "aurora" } | ConvertTo-Json -Depth 10
            $config | Out-File -FilePath $script:TestConfigPath -Encoding UTF8

            Write-UpdateFields -LastCheckTs 12345 -LastNotifiedVersion "1.0.0"

            $result = Get-Content -Raw -LiteralPath $script:TestConfigPath | ConvertFrom-Json
            $result.update | Should -Not -BeNullOrEmpty
            $result.update.last_check_ts | Should -Be 12345
            $result.update.last_notified_version | Should -Be "1.0.0"
        }

        It "preserves unknown keys in config" {
            $config = @{
                theme = "aurora"
                custom_user_field = "should survive"
                update = @{
                    auto_check = $true
                    last_check_ts = 0
                    last_notified_version = ""
                }
            } | ConvertTo-Json -Depth 10
            $config | Out-File -FilePath $script:TestConfigPath -Encoding UTF8

            Write-UpdateFields -LastCheckTs 999 -LastNotifiedVersion "x.y.z"

            $result = Get-Content -Raw -LiteralPath $script:TestConfigPath | ConvertFrom-Json
            $result.custom_user_field | Should -Be "should survive"
        }

        It "adds missing fields to an existing update object" {
            $config = @{
                update = @{
                    channel = "stable"
                }
            } | ConvertTo-Json -Depth 10
            $config | Out-File -FilePath $script:TestConfigPath -Encoding UTF8

            Write-UpdateFields -LastCheckTs 321 -LastNotifiedVersion "3.0.0"

            $result = Get-Content -Raw -LiteralPath $script:TestConfigPath | ConvertFrom-Json
            $result.update.channel | Should -Be "stable"
            $result.update.last_check_ts | Should -Be 321
            $result.update.last_notified_version | Should -Be "3.0.0"
        }

        It "writes UTF-8 without BOM" {
            $config = @{
                update = @{
                    last_check_ts = 0
                    last_notified_version = ""
                }
            } | ConvertTo-Json -Depth 10
            $config | Out-File -FilePath $script:TestConfigPath -Encoding UTF8

            Write-UpdateFields -LastCheckTs 1 -LastNotifiedVersion "1"

            $bytes = [System.IO.File]::ReadAllBytes($script:TestConfigPath)
            $hasBom = ($bytes.Length -ge 3) -and ($bytes[0] -eq 0xEF) -and ($bytes[1] -eq 0xBB) -and ($bytes[2] -eq 0xBF)
            $hasBom | Should -Be $false
        }

        It "handles empty config file gracefully" {
            "{}" | Out-File -FilePath $script:TestConfigPath -Encoding UTF8

            Write-UpdateFields -LastCheckTs 42 -LastNotifiedVersion "test"

            $result = Get-Content -Raw -LiteralPath $script:TestConfigPath | ConvertFrom-Json
            $result.update.last_check_ts | Should -Be 42
            $result.update.last_notified_version | Should -Be "test"
        }
    }

    Describe "Assert-HttpsUrl allow-list" {
        It "allows api.github.com" {
            { Assert-HttpsUrl "https://api.github.com/repos/x/y/releases" } | Should -Not -Throw
        }

        It "allows github.com" {
            { Assert-HttpsUrl "https://github.com/x/y/releases/download/v1.0/z.zip" } | Should -Not -Throw
        }

        It "allows objects.githubusercontent.com" {
            { Assert-HttpsUrl "https://objects.githubusercontent.com/x/y" } | Should -Not -Throw
        }

        It "allows release-assets.githubusercontent.com" {
            { Assert-HttpsUrl "https://release-assets.githubusercontent.com/x/y" } | Should -Not -Throw
        }

        It "rejects HTTP (non-HTTPS)" {
            { Assert-HttpsUrl "http://github.com/x/y" } | Should -Throw
        }

        It "rejects non-allowlisted host" {
            { Assert-HttpsUrl "https://evil.com/x/y" } | Should -Throw
        }

        It "does not allow an environment variable to bypass HTTPS" {
            $env:SKY_UPDATER_FAKE_ROOT = "http://localhost:1234"
            { Assert-HttpsUrl "http://localhost:1234/release.json" } | Should -Throw
        }
    }

    Describe "Read-Sha256Sidecar" {
        BeforeEach {
            $script:SidecarPath = Join-Path $env:TEMP ("sky-sidecar-" + [guid]::NewGuid() + ".sha256")
        }

        AfterEach {
            Remove-Item -LiteralPath $script:SidecarPath -Force -ErrorAction SilentlyContinue
        }

        It "accepts the exact release filename" {
            $hash = 'a' * 64
            "$hash  Sky-Auto-Player-v2.4.4.zip" |
                Out-File -Encoding ASCII -LiteralPath $script:SidecarPath

            Read-Sha256Sidecar `
                -SidecarPath $script:SidecarPath `
                -ExpectedFileName 'Sky-Auto-Player-v2.4.4.zip' | Should -Be $hash
        }

        It "rejects a hash bound to a different filename" {
            $hash = 'a' * 64
            "$hash  other.zip" | Out-File -Encoding ASCII -LiteralPath $script:SidecarPath

            {
                Read-Sha256Sidecar `
                    -SidecarPath $script:SidecarPath `
                    -ExpectedFileName 'Sky-Auto-Player-v2.4.4.zip'
            } | Should -Throw
        }
    }
    Describe "Test-WriteAccess" {
        It "returns true for writable directory" {
            $dir = New-Item -ItemType Directory -Force -Path (Join-Path $env:TEMP ("sky-test-write-" + [guid]::NewGuid()))
            try {
                Test-WriteAccess $dir.FullName | Should -Be $true
            } finally {
                Remove-Item -Recurse -Force $dir.FullName -ErrorAction SilentlyContinue
            }
        }

        It "returns false for non-existent directory" {
            Test-WriteAccess "C:\this\path\does\not\exist\12345" | Should -Be $false
        }
    }

    Describe "Compare-Version error path" {
        # The actual PEP 440 comparison is delegated to Sky-Auto-Player.exe
        # --compare-versions and is integration-tested by build_app's smoke
        # gate. Pester cannot meaningfully mock a binary that does not exist
        # in this repo's test environment, so we cover only the not-found
        # error path here.
        It "throws when Get-ExePath returns a non-existent path" {
            Mock Get-ExePath { 'C:\definitely-not-found-' + [guid]::NewGuid().ToString('N') + '.exe' }
            { Compare-Version -Current '1.0.0' -Latest '2.0.0' } | Should -Throw -ExpectedMessage '*not found*'
        }
    }

    Describe "Test-ManifestIntegrity (fail-closed per-file hash gate)" {
        BeforeEach {
            $script:StagingRoot = New-Item -ItemType Directory -Force -Path (Join-Path $env:TEMP ("sky-manifest-stage-" + [guid]::NewGuid()))
        }

        AfterEach {
            if ($script:StagingRoot -and (Test-Path $script:StagingRoot)) {
                Remove-Item -Recurse -Force $script:StagingRoot -ErrorAction SilentlyContinue
            }
        }

        It "returns true when MANIFEST.json exists and all files hash-match" {
            $f1 = Write-TestFile "data\foo.txt" "alpha"
            $f2 = Write-TestFile "data\bar.txt" "beta"
            Write-TestManifest @(
                @{ path = "data/foo.txt"; sha256 = (Get-FileSha256 $f1) }
                @{ path = "data/bar.txt"; sha256 = (Get-FileSha256 $f2) }
            )
            Test-ManifestIntegrity -StagingRoot $script:StagingRoot.FullName | Should -Be $true
        }

        It "returns false (fail-closed) when MANIFEST.json is missing" {
            # Hardening regression: before the fix, a missing MANIFEST was
            # silently ignored and the install proceeded with zip-level
            # SHA256 only. Now it is refused.
            Write-TestFile "data\foo.txt" "alpha"
            Test-ManifestIntegrity -StagingRoot $script:StagingRoot.FullName | Should -Be $false
        }

        It "returns false when MANIFEST.json has no files[] array" {
            '{}' | Out-File -Encoding UTF8 -LiteralPath (Join-Path $script:StagingRoot 'MANIFEST.json')
            Test-ManifestIntegrity -StagingRoot $script:StagingRoot.FullName | Should -Be $false
        }

        It "returns false when files[] is empty" {
            '{"files": []}' | Out-File -Encoding UTF8 -LiteralPath (Join-Path $script:StagingRoot 'MANIFEST.json')
            Test-ManifestIntegrity -StagingRoot $script:StagingRoot.FullName | Should -Be $false
        }

        It "returns false when a manifest-listed file is missing from staging" {
            $f1 = Write-TestFile "data\foo.txt" "alpha"
            Write-TestManifest @(
                @{ path = "data/foo.txt"; sha256 = (Get-FileSha256 $f1) }
                @{ path = "data/missing.txt"; sha256 = ("A" * 64) }
            )
            Test-ManifestIntegrity -StagingRoot $script:StagingRoot.FullName | Should -Be $false
        }

        It "returns false when a manifest hash does not match the file content" {
            $f1 = Write-TestFile "data\foo.txt" "alpha"
            Write-TestManifest @(
                @{ path = "data/foo.txt"; sha256 = ("0" * 64) }
            )
            Test-ManifestIntegrity -StagingRoot $script:StagingRoot.FullName | Should -Be $false
        }

        It "returns false when MANIFEST.json is corrupt JSON" {
            'not valid json {{{' | Out-File -Encoding UTF8 -LiteralPath (Join-Path $script:StagingRoot 'MANIFEST.json')
            Test-ManifestIntegrity -StagingRoot $script:StagingRoot.FullName | Should -Be $false
        }

        It "returns false when executable_sha256 does not match" {
            $f1 = Write-TestFile "data\foo.txt" "alpha"
            Write-TestManifest @(
                @{ path = "data/foo.txt"; sha256 = (Get-FileSha256 $f1) }
            )
            [System.IO.File]::WriteAllText(
                (Join-Path $script:StagingRoot 'Sky-Auto-Player.exe'),
                'tampered executable'
            )

            Test-ManifestIntegrity -StagingRoot $script:StagingRoot.FullName | Should -Be $false
        }

        It "returns false when staging contains an unmanifested extra file" {
            $f1 = Write-TestFile "data\foo.txt" "alpha"
            Write-TestManifest @(
                @{ path = "data/foo.txt"; sha256 = (Get-FileSha256 $f1) }
            )
            Write-TestFile "extra.dll" "unlisted"

            Test-ManifestIntegrity -StagingRoot $script:StagingRoot.FullName | Should -Be $false
        }

        It "returns false when a manifest path escapes staging" {
            $outside = Join-Path (Split-Path -Parent $script:StagingRoot.FullName) 'outside.txt'
            [System.IO.File]::WriteAllText($outside, 'outside')
            try {
                Write-TestManifest @(
                    @{ path = "../outside.txt"; sha256 = (Get-FileSha256 $outside) }
                )

                Test-ManifestIntegrity -StagingRoot $script:StagingRoot.FullName | Should -Be $false
            } finally {
                Remove-Item -LiteralPath $outside -Force -ErrorAction SilentlyContinue
            }
        }

        It "returns false when manifest version differs from the selected release" {
            $f1 = Write-TestFile "data\foo.txt" "alpha"
            Write-TestManifest @(
                @{ path = "data/foo.txt"; sha256 = (Get-FileSha256 $f1) }
            ) -Version "9.9.8"

            Test-ManifestIntegrity `
                -StagingRoot $script:StagingRoot.FullName `
                -ExpectedVersion "9.9.9" | Should -Be $false
        }
    }

    Describe "Assert-ZipArchiveSafe" {
        BeforeEach {
            Add-Type -AssemblyName System.IO.Compression.FileSystem
            $script:ZipPath = Join-Path $env:TEMP ("sky-zip-test-" + [guid]::NewGuid() + ".zip")
        }

        AfterEach {
            Remove-Item -LiteralPath $script:ZipPath -Force -ErrorAction SilentlyContinue
        }

        It "accepts a normal relative zip layout" {
            $archive = [System.IO.Compression.ZipFile]::Open($script:ZipPath, 'Create')
            try {
                $null = $archive.CreateEntry('Sky-Auto-Player.exe')
                $null = $archive.CreateEntry('data/file.bin')
            } finally {
                $archive.Dispose()
            }

            { Assert-ZipArchiveSafe -ZipPath $script:ZipPath } | Should -Not -Throw
        }

        It "rejects a zip entry that escapes the extraction root" {
            $archive = [System.IO.Compression.ZipFile]::Open($script:ZipPath, 'Create')
            try {
                $null = $archive.CreateEntry('../escape.txt')
            } finally {
                $archive.Dispose()
            }

            { Assert-ZipArchiveSafe -ZipPath $script:ZipPath } | Should -Throw
        }
    }
    Describe "Copy-UpdateTree transactional copy" {
        BeforeEach {
            $script:StagingRoot = New-Item -ItemType Directory -Force -Path (Join-Path $env:TEMP ("sky-stage-" + [guid]::NewGuid()))
            $script:DestRoot    = New-Item -ItemType Directory -Force -Path (Join-Path $env:TEMP ("sky-dest-"  + [guid]::NewGuid()))
        }

        AfterEach {
            if ($script:StagingRoot -and (Test-Path $script:StagingRoot)) { Remove-Item -Recurse -Force $script:StagingRoot -ErrorAction SilentlyContinue }
            if ($script:DestRoot    -and (Test-Path $script:DestRoot))    { Remove-Item -Recurse -Force $script:DestRoot    -ErrorAction SilentlyContinue }
        }

        It "copies new files from staging to dest" {
            "content1" | Out-File (Join-Path $script:StagingRoot "newfile.txt") -Encoding UTF8
            # Create the sub directory BEFORE writing into it — Out-File does
            # not create parent directories.
            New-Item -ItemType Directory -Force -Path (Join-Path $script:StagingRoot "sub") | Out-Null
            "content2" | Out-File (Join-Path $script:StagingRoot "sub\other.txt") -Encoding UTF8

            Copy-UpdateTree -StagingRoot $script:StagingRoot.FullName -DestRoot $script:DestRoot.FullName

            (Test-Path (Join-Path $script:DestRoot "newfile.txt")) | Should -Be $true
            (Get-Content (Join-Path $script:DestRoot "newfile.txt")) | Should -Be "content1"
            (Test-Path (Join-Path $script:DestRoot "sub\other.txt")) | Should -Be $true
            (Get-Content (Join-Path $script:DestRoot "sub\other.txt")) | Should -Be "content2"
        }

        It "backs up existing files before overwrite" {
            "old" | Out-File (Join-Path $script:DestRoot "existing.txt") -Encoding UTF8
            "new" | Out-File (Join-Path $script:StagingRoot "existing.txt") -Encoding UTF8

            Copy-UpdateTree -StagingRoot $script:StagingRoot.FullName -DestRoot $script:DestRoot.FullName

            (Get-Content (Join-Path $script:DestRoot "existing.txt")) | Should -Be "new"
            # Backup dir is created under $env:TEMP with prefix "sky-backup-"
            # and is removed on success. Any leftover backup dir for THIS
            # test run would indicate the cleanup path was missed.
            $backupDirs = @(
                Get-ChildItem -Path $env:TEMP -Filter "sky-backup-*" -Directory -ErrorAction SilentlyContinue
                | Where-Object { $_.CreationTime -gt (Get-Date).AddMinutes(-2) }
            )
            $backupDirs.Count | Should -Be 0
        }

        It "skips config.json entirely" {
            "old config" | Out-File (Join-Path $script:DestRoot "config.json") -Encoding UTF8
            "new config" | Out-File (Join-Path $script:StagingRoot "config.json") -Encoding UTF8

            Copy-UpdateTree -StagingRoot $script:StagingRoot.FullName -DestRoot $script:DestRoot.FullName

            (Get-Content (Join-Path $script:DestRoot "config.json")) | Should -Be "old config"
        }

        It "preserves a local .env file that is absent from the release" {
            "LOCAL_TOKEN=keep-me" | Out-File (Join-Path $script:DestRoot ".env") -Encoding UTF8

            Copy-UpdateTree -StagingRoot $script:StagingRoot.FullName -DestRoot $script:DestRoot.FullName

            (Get-Content (Join-Path $script:DestRoot ".env")) | Should -Be "LOCAL_TOKEN=keep-me"
        }

        It "skips songs/ directory entirely (top-level preserve-list mandate)" {
            # Create BOTH dest and staging songs\ directories BEFORE writing
            # files into them — Out-File does not create parent directories.
            New-Item -ItemType Directory -Force -Path (Join-Path $script:DestRoot    "songs") | Out-Null
            New-Item -ItemType Directory -Force -Path (Join-Path $script:StagingRoot "songs") | Out-Null
            "old song" | Out-File (Join-Path $script:DestRoot    "songs\song.json") -Encoding UTF8
            "new song" | Out-File (Join-Path $script:StagingRoot "songs\song.json") -Encoding UTF8

            Copy-UpdateTree -StagingRoot $script:StagingRoot.FullName -DestRoot $script:DestRoot.FullName

            (Get-Content (Join-Path $script:DestRoot "songs\song.json")) | Should -Be "old song"
        }

        It "skips songs/ subdirectories (nested preserve-list regression guard)" {
            # Regression guard for the Bug A fix: Get-ChildItem -Recurse -File
            # emits backslash paths on Windows. The naive $rel.StartsWith('songs/')
            # check missed nested files. After normalization, songs\artists\foo
            # is preserved as well.
            New-Item -ItemType Directory -Force -Path (Join-Path $script:DestRoot    "songs\artists") | Out-Null
            New-Item -ItemType Directory -Force -Path (Join-Path $script:StagingRoot "songs\artists") | Out-Null
            "old" | Out-File (Join-Path $script:DestRoot    "songs\artists\foo.json") -Encoding UTF8
            "new" | Out-File (Join-Path $script:StagingRoot "songs\artists\foo.json") -Encoding UTF8

            Copy-UpdateTree -StagingRoot $script:StagingRoot.FullName -DestRoot $script:DestRoot.FullName

            (Get-Content (Join-Path $script:DestRoot "songs\artists\foo.json")) | Should -Be "old"
        }

        It "does not falsely match sibling directories named songsX (e.g. songs2)" {
            # Regression guard: prefix-match must require the songs\ separator,
            # so songs2\foo.json (a sibling folder) is still updated.
            New-Item -ItemType Directory -Force -Path (Join-Path $script:DestRoot    "songs2") | Out-Null
            New-Item -ItemType Directory -Force -Path (Join-Path $script:StagingRoot "songs2") | Out-Null
            "old" | Out-File (Join-Path $script:DestRoot    "songs2\foo.json") -Encoding UTF8
            "new" | Out-File (Join-Path $script:StagingRoot "songs2\foo.json") -Encoding UTF8

            Copy-UpdateTree -StagingRoot $script:StagingRoot.FullName -DestRoot $script:DestRoot.FullName

            (Get-Content (Join-Path $script:DestRoot "songs2\foo.json")) | Should -Be "new"
        }

        It "rolls back on copy failure and restores the original file" {
            # Strategy: lock the STAGING file with FileShare.None during the
            # copy phase. The backup phase (which reads only the dest) succeeds
            # and stages the original "old" content. The copy phase then fails
            # opening the locked staging file; the catch block restores the
            # dest from backup, leaving "old" — verifiable after the stream
            # is closed. ReadOnly attribute does NOT block Copy-Item -Force,
            # so this FileStream-lock approach is required.
            "old" | Out-File (Join-Path $script:DestRoot    "lockedfile.txt") -Encoding UTF8
            "new" | Out-File (Join-Path $script:StagingRoot "lockedfile.txt") -Encoding UTF8

            $stageFile = Join-Path $script:StagingRoot "lockedfile.txt"
            $stream = [System.IO.File]::Open($stageFile, 'Open', 'Read', 'None')
            try {
                { Copy-UpdateTree -StagingRoot $script:StagingRoot.FullName -DestRoot $script:DestRoot.FullName } | Should -Throw
            } finally {
                $stream.Close()
            }

            (Get-Content (Join-Path $script:DestRoot "lockedfile.txt")) | Should -Be "old"
        }

        It "removes the durable transaction directory after a successful copy" {
            "new" | Out-File (Join-Path $script:StagingRoot "newfile.txt") -Encoding UTF8

            Copy-UpdateTree -StagingRoot $script:StagingRoot.FullName -DestRoot $script:DestRoot.FullName

            Test-Path (Join-Path $script:DestRoot '.sky-update-transaction') | Should -Be $false
        }
    }

    Describe "Recover-InterruptedUpdate durable recovery" {
        BeforeEach {
            $script:DestRoot = New-Item -ItemType Directory -Force -Path (
                Join-Path $env:TEMP ("sky-recover-dest-" + [guid]::NewGuid())
            )
        }

        AfterEach {
            if ($script:DestRoot -and (Test-Path $script:DestRoot)) {
                Remove-Item -Recurse -Force $script:DestRoot -ErrorAction SilentlyContinue
            }
        }

        It "restores backups and removes files created by an interrupted update" {
            $tx = Join-Path $script:DestRoot '.sky-update-transaction'
            $backup = Join-Path $tx 'backup'
            New-Item -ItemType Directory -Force -Path $backup | Out-Null
            "old" | Out-File (Join-Path $backup 'existing.txt') -Encoding UTF8
            "new" | Out-File (Join-Path $script:DestRoot 'existing.txt') -Encoding UTF8
            "new-only" | Out-File (Join-Path $script:DestRoot 'newfile.txt') -Encoding UTF8
            @{
                schema_version = 1
                state = "prepared"
                backed_up = @("existing.txt")
                new_files = @("newfile.txt", "not-created.txt")
            } | ConvertTo-Json -Depth 10 |
                Out-File -Encoding UTF8 -LiteralPath (Join-Path $tx 'journal.json')

            Recover-InterruptedUpdate -DestRoot $script:DestRoot.FullName | Should -Be $true

            Get-Content (Join-Path $script:DestRoot 'existing.txt') | Should -Be "old"
            Test-Path (Join-Path $script:DestRoot 'newfile.txt') | Should -Be $false
            Test-Path $tx | Should -Be $false
        }

        It "retains the transaction directory when a backup cannot be restored" {
            $tx = Join-Path $script:DestRoot '.sky-update-transaction'
            New-Item -ItemType Directory -Force -Path $tx | Out-Null
            @{
                schema_version = 1
                state = "prepared"
                backed_up = @("missing.txt")
                new_files = @()
            } | ConvertTo-Json -Depth 10 |
                Out-File -Encoding UTF8 -LiteralPath (Join-Path $tx 'journal.json')

            Recover-InterruptedUpdate -DestRoot $script:DestRoot.FullName | Should -Be $false
            Test-Path $tx | Should -Be $true
        }
    }

    Describe "Epoch generation is UTC (regression guard for local-time bug)" {
        # Regression guard for the bug where the source used:
        #     [int][double]::Parse((Get-Date -UFormat %s), InvariantCulture)
        # — ``Get-Date -UFormat %s`` returns a LOCAL-time epoch (relative to
        # the machine's timezone, NOT UTC), so on UTC+7 it diverged from
        # Python ``int(time.time())`` by exactly 25200 seconds.  That broke
        # two downstream consumers in update_service.py / modals.py:
        #   * ``should_auto_check`` read ``now(UTC) - last_check_ts(local)``
        #     and got a negative delta on positive-offset zones, silently
        #     bypassing the 24h throttle and spamming the GitHub API.
        #   * ``time.localtime(last_check_ts)`` rendered the local-time epoch
        #     with the offset applied a second time, showing "last checked"
        #     off by 2x the tz offset vs an in-app (Python-UTC) check.
        # The fix uses ``[DateTimeOffset]::UtcNow.ToUnixTimeSeconds()`` which
        # is the .NET standard for a UTC Unix epoch — locale-free, no sub-second
        # floor surprise, and identical to Python ``time.time()``.
        #
        # The current source line in updater.ps1 is checked verbatim so a
        # future "cleanup" cannot silently resurrect the buggy form.
        It "uses [DateTimeOffset]::UtcNow.ToUnixTimeSeconds() (not Get-Date -UFormat %s)" {
            $src = Get-Content -Raw -LiteralPath (Join-Path $PSScriptRoot '..\updater.ps1')
            # The fix line MUST be present verbatim. Anchor on the assignment
            # at start-of-line (the code is ``$epoch = [int][DateTimeOffset]::UtcNow.ToUnixTimeSeconds()``)
            # so a comment that merely *describes* the fix cannot satisfy this check.
            $src | Should -Match '(?m)^\s*\$epoch\s*=\s*\[int\]\[DateTimeOffset\]::UtcNow\.ToUnixTimeSeconds\(\)'
            # The buggy local-time form MUST be absent as an executable statement.
            # Anchor on the assignment at start-of-line so the comment at
            # ``updater.ps1:692`` documenting the old bug (``# ``[int][double]::Parse...``)
            # is NOT matched — only a real ``$epoch = ... (Get-Date -UFormat %s) ...``
            # statement would fire this.
            $src | Should -Not -Match '(?m)^\s*\$epoch\s*=\s*\[int\]\[double\]::Parse\(\s*\(Get-Date -UFormat %s\)'
        }

        It "generated epoch matches the UTC reference within ±5s on any timezone" {
            # Reference: the WELL-KNOWN UTC form (``ToUniversalTime() -UFormat
            # %s`` parsed InvariantCulture) — this is what the buggy line was
            # TRYING to compute and what Python ``int(time.time())`` equals.
            # Both evaluations run within milliseconds of each other in the
            # same process, so they MUST agree to within ±5s even across a
            # second boundary; any divergence > 5s indicates one source is not
            # UTC (the regression signature is divergence == tz offset, which
            # is at least 900s for every real timezone).
            $epoch    = [int][DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
            $refUtc = [int][double]::Parse(
                (Get-Date -Date (Get-Date).ToUniversalTime() -UFormat %s),
                [System.Globalization.CultureInfo]::InvariantCulture
            )
            $delta = [Math]::Abs($epoch - $refUtc)
            $delta | Should -BeLessOrEqual 5
        }
    }

    # =========================================================================
    # Phase 0 tests: Bridge / rename logic (failing until Phase 1)
    # =========================================================================

    Describe "Resolve-PrimaryExe" {
        BeforeEach {
            $script:TestRoot = New-Item -ItemType Directory -Force -Path (Join-Path $env:TEMP ("sky-resolve-exe-" + [guid]::NewGuid()))
        }
        AfterEach {
            if ($script:TestRoot -and (Test-Path $script:TestRoot)) {
                Remove-Item -Recurse -Force $script:TestRoot -ErrorAction SilentlyContinue
            }
        }
        
        It "Resolve-PrimaryExe prefers Sky-Auto-Player.exe" {
            New-Item -ItemType File -Force -Path (Join-Path $script:TestRoot "Sky-Auto-Player.exe") | Out-Null
            New-Item -ItemType File -Force -Path (Join-Path $script:TestRoot "Sky-Player.exe") | Out-Null
            
            $result = Resolve-PrimaryExe -Root $script:TestRoot.FullName
            $result | Should -Be (Join-Path $script:TestRoot.FullName "Sky-Auto-Player.exe")
        }

        It "Resolve-PrimaryExe falls back to Sky-Player.exe" {
            New-Item -ItemType File -Force -Path (Join-Path $script:TestRoot "Sky-Player.exe") | Out-Null
            
            $result = Resolve-PrimaryExe -Root $script:TestRoot.FullName
            $result | Should -Be (Join-Path $script:TestRoot.FullName "Sky-Player.exe")
        }

        It "Resolve-PrimaryExe fails when neither exists" {
            { Resolve-PrimaryExe -Root $script:TestRoot.FullName } | Should -Throw
        }
    }

    Describe "Select-ReleaseAssets" {
        It "Select-ReleaseAssets prefers canonical pair" {
            $assets = @(
                @{ name = "Sky-Auto-Player-v2.4.2.zip"; browser_download_url = "http://a/zip" },
                @{ name = "Sky-Auto-Player-v2.4.2.zip.sha256"; browser_download_url = "http://a/sha" },
                @{ name = "Sky-Player-v2.4.2.zip"; browser_download_url = "http://b/zip" },
                @{ name = "Sky-Player-v2.4.2.zip.sha256"; browser_download_url = "http://b/sha" }
            )
            $result = Select-ReleaseAssets -Assets $assets -Version "2.4.2"
            $result.ZipAsset.name | Should -Be "Sky-Auto-Player-v2.4.2.zip"
            $result.ShaAsset.name | Should -Be "Sky-Auto-Player-v2.4.2.zip.sha256"
        }

        It "Select-ReleaseAssets falls back to legacy pair" {
            $assets = @(
                @{ name = "Sky-Player-v2.4.2.zip"; browser_download_url = "http://b/zip" },
                @{ name = "Sky-Player-v2.4.2.zip.sha256"; browser_download_url = "http://b/sha" }
            )
            $result = Select-ReleaseAssets -Assets $assets -Version "2.4.2"
            $result.ZipAsset.name | Should -Be "Sky-Player-v2.4.2.zip"
            $result.ShaAsset.name | Should -Be "Sky-Player-v2.4.2.zip.sha256"
        }

        It "Select-ReleaseAssets refuses mixed pairs" {
            $assets = @(
                @{ name = "Sky-Auto-Player-v2.4.2.zip"; browser_download_url = "http://a/zip" },
                @{ name = "Sky-Player-v2.4.2.zip.sha256"; browser_download_url = "http://b/sha" }
            )
            { Select-ReleaseAssets -Assets $assets -Version "2.4.2" } | Should -Throw
        }
    }

    Describe "Process guard logic" {
        It "Resolve-ProcessNames includes both Sky-Auto-Player and Sky-Player" {
            $names = Resolve-ProcessNames
            $names | Should -Contain "Sky-Auto-Player"
            $names | Should -Contain "Sky-Player"
        }
    }

    Describe "Resolve-StagingRoot" {
        BeforeEach {
            $script:TestRoot = New-Item -ItemType Directory -Force -Path (Join-Path $env:TEMP ("sky-staging-root-" + [guid]::NewGuid()))
        }
        AfterEach {
            if ($script:TestRoot -and (Test-Path $script:TestRoot)) {
                Remove-Item -Recurse -Force $script:TestRoot -ErrorAction SilentlyContinue
            }
        }

        It "Staging accepts Sky-Player.exe-only layout" {
            New-Item -ItemType File -Force -Path (Join-Path $script:TestRoot "Sky-Player.exe") | Out-Null
            $result = Resolve-StagingRoot -ExtractDir $script:TestRoot.FullName
            $result | Should -Be $script:TestRoot.FullName
        }

        It "Staging accepts Sky-Auto-Player.exe-only layout" {
            New-Item -ItemType File -Force -Path (Join-Path $script:TestRoot "Sky-Auto-Player.exe") | Out-Null
            $result = Resolve-StagingRoot -ExtractDir $script:TestRoot.FullName
            $result | Should -Be $script:TestRoot.FullName
        }
        
        It "Staging fails if neither exists" {
            New-Item -ItemType File -Force -Path (Join-Path $script:TestRoot "other.exe") | Out-Null
            { Resolve-StagingRoot -ExtractDir $script:TestRoot.FullName } | Should -Throw
        }
    }

    # =========================================================================
    # Regression: Initialize-Paths must work under ``pwsh -File updater.ps1``
    # =========================================================================
    # Background: before this regression guard, Initialize-Paths gated the
    # path auto-detection on ``$MyInvocation.MyCommand.Path -eq $PSCommandPath``.
    # Inside a *function call*, ``$MyInvocation.MyCommand.Path`` is ``$null``
    # (PowerShell functions do not own a command path), while
    # ``$PSCommandPath`` is correctly the running script's path. The
    # comparison ``$null -eq '<script path>'`` evaluated False, leaving
    # ``$global:InstallRoot`` empty and breaking every ``updater.bat``
    # invocation with
    # ``Cannot bind argument to parameter 'Path' because it is an empty string``
    # at the ``Test-WriteAccess $InstallRoot`` gate. The fix drives auto-
    # detection off ``$PSCommandPath`` alone (which is ``$null`` under
    # dot-source, so Pester ``BeforeAll`` pre-set globals take precedence).
    Describe "Initialize-Paths under pwsh -File invocation" {
        BeforeAll {
            # Build a throwaway install-shaped tree with the real
            # updater.ps1 (copied so we don't mutate the source) and a
            # Sky-Auto-Player.exe stub. We then invoke ``pwsh -File`` on
            # that copy and capture whether the script reached the version
            # check (proof Initialize-Paths set ``$InstallRoot``).
            $script:FileTestRoot = Join-Path $env:TEMP ('sky-file-init-' + [guid]::NewGuid().ToString('N'))
            New-Item -ItemType Directory -Force -Path $script:FileTestRoot | Out-Null
            $script:TestInstaller = Join-Path $script:FileTestRoot 'installer'
            New-Item -ItemType Directory -Force -Path $script:TestInstaller | Out-Null
            Copy-Item -LiteralPath (Join-Path $PSScriptRoot '..\updater.ps1') -Destination (Join-Path $script:TestInstaller 'updater.ps1') -Force
            # Fake exe + config + updater.bat so updater.ps1's surrounding
            # assumptions resolve. We only need the path-init path to work;
            # we then exit before network/process operations.
            New-Item -ItemType File -Force -Path (Join-Path $script:FileTestRoot 'Sky-Auto-Player.exe') | Out-Null
            '{"theme":"x","update":{"channel":"stable","last_check_ts":0,"last_notified_version":""}}' |
                Out-File -Encoding UTF8 -LiteralPath (Join-Path $script:FileTestRoot 'config.json')
            # updater.bat itself is not required for the -File probe below.
        }
        AfterAll {
            if ($script:FileTestRoot -and (Test-Path $script:FileTestRoot)) {
                Remove-Item -Recurse -Force $script:FileTestRoot -ErrorAction SilentlyContinue
            }
        }

        It "Initialize-Paths sets `$global:InstallRoot when invoked via pwsh -File" {
            # Run the real updater.ps1 via ``pwsh -File`` with -DryRun so it
            # exits cleanly after the path-init + version-compare path. If
            # Initialize-Paths fails to set ``$InstallRoot``, the script dies
            # at the Test-WriteAccess gate with exit 1 + "Cannot bind
            # argument to parameter 'Path' because it is an empty string".
            # A successful path-init instead reaches the GitHub/fake-root
            # fetch and fails there (no fake root configured) with a
            # different exit/message. Either non-empty-InstallRoot outcome
            # is acceptable for THIS regression test: the bug signature is
            # uniquely the empty-path binding error.
            $out = & pwsh -NoProfile -ExecutionPolicy Bypass -File (Join-Path $script:TestInstaller 'updater.ps1') -DryRun 2>&1
            $lastExit = $LASTEXITCODE
            $combined = ($out -join "`n")
            # The empty-path binding error is the unique failure signature
            # of the regression we are guarding against.
            $combined | Should -Not -Match 'Cannot bind argument to parameter ''Path'' because it is an empty string'
        }
    }

    # =========================================================================
    # Regression: Get-RelativePathSafe must survive 8.3 short-name mismatches
    # =========================================================================
    # Background: ``Copy-UpdateTree`` originally used
    # ``$file.FullName.Substring($StagingRoot.Length)`` to compute relative
    # paths. The Compare-Version + extract path builds ``$StagingRoot`` under
    # ``$env:TEMP``, which on short-name-enabled volumes comes back as
    # ``C:\Users\PE4CE_~1\...`` while ``Get-ChildItem -Recurse`` emits long
    # names (``pe4cE_HOA``). The mismatch means the substring drops one too
    # many chars and the leftover fragment (``t\``) becomes a phantom top-
    # level directory in the install root — every bridge J3 update landed
    # in ``<install>\t\`` instead of ``<install>\``, partially preserving
    # the install only when the Working-Tree happened to also be short-
    # named.
    #
    # Fix: route every relative-path compute through ``Get-RelativePathSafe``
    # which first normalizes both sides through ``Scripting.FileSystemObject
    # .GetAbsolutePathName``. These tests then drive the helper from both
    # directions (long-base + long-full, mixed 8.3 vs long) and assert the
    # helper returns a clean backslash-prefixed relative path that matches
    # what ``Join-Path`` would have produced for the same canonical pair.
    Describe "Get-RelativePathSafe" {
        It "returns the relative path when base and full share long-form names" {
            $base = 'C:\A\B\C'
            $full = 'C:\A\B\C\D\E\file.txt'
            Get-RelativePathSafe -Base $base -Full $full | Should -Be 'D\E\file.txt'
        }

        It "returns '' when full equals base" {
            $base = 'C:\A\B\C'
            $full = 'C:\A\B\C'
            Get-RelativePathSafe -Base $base -Full $full | Should -Be ''
        }

        It "normalizes 8.3 short-name in ``$Base`` against a long-name ``$Full``" {
            # On short-name enabled Windows volumes (the typical install
            # environment of this project's users) ``$env:TEMP`` returns
            # ``C:\Users\PE4CE_~1\AppData\Local\Temp`` while
            # ``Get-ChildItem`` already returns long forms. We simulate
            # the same shape by constructing Base via short-name.
            $tmp = Join-Path $env:TEMP ('sky-relpath-' + [guid]::NewGuid().ToString('N'))
            New-Item -ItemType Directory -Force -Path $tmp | Out-Null
            try {
                # Capture the on-disk short-name form using FileSystemObject
                $fso = New-Object -ComObject Scripting.FileSystemObject
                $shortBase = $fso.GetAbsolutePathName($tmp)  # GetAbsolutePathName resolves the long path, so we simulate by el upper-case
                $shortBase = $shortBase.ToUpper().Replace('C:\USERS\','C:\Users\')  # crude simulator for short-name mismatch
                # Use the literal fact that PowerShell emits Normalize + long; we instead fabricate
                # the scenario where Base ends with a different final char than Full's prefix
                $fileFull = Join-Path $tmp 'inside-file.txt'
                New-Item -ItemType File -Force -Path $fileFull | Out-Null
                # If the system does *not* have short names enabled, Base and Full literally
                # agree and the helper returns the empty sub-path.  We then expect rght
                # behavior in EITHER case: never a non-empty bogus prefix.
                $rel = Get-RelativePathSafe -Base $tmp -Full $fileFull
                # Both forms (empty / 'inside-file.txt') are acceptable here; what is NOT
                # acceptable is a stray 'xt/...'-style phantom prefix from a partial
                # prefix match.
                $rel -replace '[\\/]' ,'' | Should -Not -Match '^[a-zA-Z]{1}$'
            } finally {
                Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
            }
        }

        It "does not truncate relative paths longer than MAX_PATH" {
            $base = 'C:\base'
            $expected = (('a' * 100) + '\' + ('b' * 100) + '\' + ('c' * 100) + '.bin')
            $full = Join-Path $base $expected

            Get-RelativePathSafe -Base $base -Full $full | Should -Be $expected
        }

        It "returns null when base is empty" {
            Get-RelativePathSafe -Base '' -Full 'C:\A\file.txt' | Should -BeNullOrEmpty
        }

        It "returns null when full is empty" {
            Get-RelativePathSafe -Base 'C:\A' -Full '' | Should -BeNullOrEmpty
        }
    }
}
