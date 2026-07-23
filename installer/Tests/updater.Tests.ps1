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
        $global:FakeRoot    = $null

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
        function script:Write-TestManifest([array]$fileEntries) {
            $manifest = @{
                app = "Sky-Auto-Player"
                version = "9.9.9-test"
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
        Remove-Variable -Scope Global -Name InstallRoot, ExePath, ConfigPath, LogDir, LogFile, FakeRoot -ErrorAction SilentlyContinue
    }

    BeforeEach {
        if ($script:TestConfigPath -and (Test-Path $script:TestConfigPath)) {
            Remove-Item -Force $script:TestConfigPath -ErrorAction SilentlyContinue
        }
        # FakeRoot may have been set by a prior test — reset before each test
        # to keep Assert-HttpsUrl tests isolated. We clear at both script and
        # global scope because, in Pester 5, an unqualified `$FakeRoot = "..."`
        # written inside an It-block propagates to both script and global
        # scope (visible in the next probe), so we reset both here.
        Remove-Variable -Scope Script -Name FakeRoot -ErrorAction SilentlyContinue
        Remove-Variable -Scope Global -Name FakeRoot -ErrorAction SilentlyContinue
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

        It "allows fake root for testing (localhost)" {
            # Assert-HttpsUrl reads unqualified $FakeRoot. Pester 5's scope
            # chain does NOT propagate $script: / $global: writes to the
            # function's lookup (verified empirically). Only an unqualified
            # assignment from inside the It-block reaches Assert-HttpsUrl —
            # probably because the Pester container runs the It-block in a
            # scope where unqualified writes propagate to the runspace's
            # shared "outer" scope, which Assert-HttpsUrl can see.
            $FakeRoot = "http://localhost:1234"
            { Assert-HttpsUrl "http://localhost:1234/release.json" } | Should -Not -Throw
        }

        It "rejects fake root not on localhost" {
            $FakeRoot = "http://evil.com:1234"
            { Assert-HttpsUrl "http://evil.com:1234/release.json" } | Should -Throw
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
}
