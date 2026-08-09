# Clean Cutover Checklist, Expected Tree and Agent Report

# A. Clean cutover checklist

## Delete script updater

- [ ] delete `updater.bat`
- [ ] delete `installer/updater.ps1`
- [ ] delete `installer/Tests/**`
- [ ] delete updater-specific PS5.1 action if unused
- [ ] delete updater-specific Pester action if unused

## Delete script packaging

From `src/build_app.py`:

- [ ] remove `REQUIRED_UPDATER_ASSETS`
- [ ] remove copy of `updater.bat`
- [ ] remove copy of `installer/`
- [ ] add `Sky-Auto-Player-Updater.exe`
- [ ] remove build-time `ExecutionPolicy Bypass` stale-process helper if present
- [ ] prefer clear locked-file failure over script-based process killing

## Delete old updater/executable compatibility

- [ ] remove `Sky-Player.exe` fallback in update architecture
- [ ] remove dual-name updater process matching
- [ ] remove dual-name staging-root logic
- [ ] remove bridge-package names
- [ ] remove tests solely for old updater migration
- [ ] remove active docs claiming old-name auto-upgrade support

Do not delete unrelated Sky game process-name logic without confirming context.

## Update active docs/UI

- [ ] `AGENTS.md`
- [ ] `docs/distribution-and-update.md`
- [ ] `docs/architecture.md` if needed
- [ ] `docs/INDEX.md`
- [ ] `README.md`
- [ ] update modal text
- [ ] English site update docs
- [ ] Vietnamese site update docs
- [ ] FAQ
- [ ] troubleshooting
- [ ] security-boundary guide
- [ ] release notes when release prep happens

New instruction:

```text
Update available → Update now
```

Remove:

```text
run updater.bat
run updater.ps1
close app and execute script
```

## Update workflows/tests

Remove:

- [ ] PS5.1 updater gate
- [ ] Pester updater gate

Add:

- [ ] `sky_updater` Rust tests
- [ ] updater build
- [ ] updater smoke
- [ ] updater dry-run/integration
- [ ] signature verification
- [ ] post-sign manifest
- [ ] package verification
- [ ] packaged upgrade E2E
- [ ] rollback E2E

## Existing-user policy

Accepted:

> Users on pre-cutover releases may need to manually download/extract the first release containing the native updater. From that release onward, the native updater is the supported update path.

Do not write compatibility code to avoid this.

Do not promise old updater compatibility.

# B. Expected source tree

```text
Sky-Auto-Player/
│
├── .github/
│   ├── actions/
│   └── workflows/
│       ├── ci.yml
│       ├── release.yml
│       └── ...
│
├── docs/
│   ├── INDEX.md
│   ├── architecture.md
│   ├── distribution-and-update.md
│   ├── rt-dispatch-architecture.md
│   ├── timing-principles.md
│   └── ...
│
├── rust/
│   ├── Cargo.toml
│   ├── Cargo.lock
│   ├── rust-toolchain.toml
│   └── crates/
│       ├── sky_dispatch_core/
│       ├── sky_dispatch_win32/
│       ├── sky_player_rs/
│       └── sky_updater/
│           ├── Cargo.toml
│           └── src/
│               ├── lib.rs
│               ├── main.rs
│               ├── cli.rs
│               ├── error.rs
│               ├── github.rs
│               ├── http.rs
│               ├── archive.rs
│               ├── manifest.rs
│               ├── signature.rs
│               ├── transaction.rs
│               ├── process.rs
│               ├── install.rs
│               ├── recovery.rs
│               ├── result.rs
│               └── restart.rs
│
├── scripts/
│   ├── build_rust_wheel.py
│   ├── build_pyinstaller_bootloader.ps1
│   ├── verify_release_signatures.ps1
│   ├── verify_release_manifest.py
│   └── ...
│
├── site/
│   └── ...
│
├── src/
│   ├── main.py
│   ├── build_app.py
│   └── sky_music/
│       ├── domain/
│       │   ├── update_checker.py
│       │   └── ...
│       ├── orchestration/
│       │   ├── update_service.py
│       │   └── ...
│       ├── infrastructure/
│       │   ├── hotkeys.py
│       │   ├── update_launcher.py
│       │   └── ...
│       ├── platform/
│       │   └── win32/
│       │       ├── global_hotkeys.py
│       │       ├── window_target.py
│       │       └── ...
│       └── ui/
│           └── ...
│
├── tests/
│   ├── fixtures/
│   │   └── updater/
│   ├── test_update_service.py
│   ├── test_update_launcher.py
│   ├── test_hotkeys.py
│   └── ...
│
├── songs/
├── config.json
├── .env.example
├── AGENTS.md
├── SECURITY.md
├── Sky-Auto-Player.spec
├── pyproject.toml
├── uv.lock
├── README.md
├── CHANGELOG.md
└── LICENSE
```

Removed from active architecture:

```text
updater.bat
installer/updater.ps1
installer/Tests/
```

If `installer/` is empty, remove it.

# C. Expected runtime install tree

```text
Sky-Auto-Player/
│
├── Sky-Auto-Player.exe
├── Sky-Auto-Player-Updater.exe
├── native_calibration.exe
├── MANIFEST.json
├── README.md
├── config.json
├── songs/
├── logs/
└── _internal/
    ├── sky_player_rs*.pyd
    └── ...
```

Must not contain:

```text
updater.bat
installer/
updater.ps1
Pester results
build cache
```

# D. Locked decisions

Do not reopen without a demonstrated blocker:

- exactly one canonical ZIP;
- no old updater compatibility;
- Rust native updater;
- out-of-process apply;
- portable app;
- no normal admin requirement;
- preserved user data;
- durable transaction + rollback;
- no AV evasion;
- Authenticode before manifest;
- PyInstaller onedir;
- UPX disabled;
- source-built matching PyInstaller bootloader.

Non-goals:

- MSI/system installer;
- Windows service;
- scheduled updater;
- registry autostart;
- update while playback remains active;
- delta patching;
- arbitrary mirrors;
- arbitrary updater URL;
- automatic elevation;
- update telemetry;
- obfuscation.

# E. Mandatory phase report template

At end of every phase report:

```text
PHASE:
STATUS: GO | NO-GO

BASE SHA:
HEAD SHA:
WORKTREE:

SCOPE COMPLETED:
- ...

FILES ADDED:
- ...

FILES MODIFIED:
- ...

FILES DELETED:
- ...

NORMATIVE DOC CHANGES:
- ...

BEHAVIORAL CHANGES:
- ...

SECURITY IMPACT:
- ...

P0 INVARIANTS:
- no game tampering:
- SendInput-only:
- no hooks:
- strict validation:

REAL-TIME INVARIANTS:
- SendInput packet semantics:
- scan-code whitelist:
- release/cleanup semantics:
- QPC/timing:
- no-allocation:
- estimator:
- wake-to-send telemetry:

UPDATE SAFETY:
- preserve config.json:
- preserve .env:
- preserve songs/:
- preserve logs/:
- outer SHA:
- archive safety:
- manifest exact-set:
- Authenticode:
- durable journal:
- rollback:
- unknown user files:

VALIDATION:
- ruff:
- pyright:
- pytest:
- security audit:
- free-threaded audit:
- cargo fmt:
- cargo check:
- cargo clippy:
- cargo test:
- frozen build:
- frozen selftests:
- updater unit:
- updater integration:
- packaged E2E:
- rollback E2E:

ARTIFACTS:
- Sky-Auto-Player.exe SHA256:
- Sky-Auto-Player-Updater.exe SHA256:
- native_calibration.exe SHA256:
- sky_player_rs.pyd SHA256:
- ZIP SHA256:
- MANIFEST schema:

SIGNING:
- enabled:
- provider:
- app signature:
- updater signature:
- calibration signature:
- pyd signature:

DEFENDER:
- Windows build:
- engine:
- intelligence:
- app:
- updater:
- pyd:
- ZIP:

KNOWN RISKS:
- ...

BLOCKERS:
- ...

FOLLOW-UP:
- ...

GO/NO-GO RATIONALE:
- ...
```

Rules:

- unknown mandatory test = `NO-GO`;
- P0 failure = `NO-GO`;
- rollback safety unproven = `NO-GO`;
- production package contains BAT/PowerShell updater = `NO-GO`;
- signing provider may be an external blocker before release phase, but not at production tag publish.
