# Phased Implementation Plan

Do not continue after `NO-GO`.

Default: one branch/PR per major phase, or at minimum clearly bounded commit series with full validation between phases.

---

# Phase 0 — Baseline and normative architecture cutover

## Reads

Read first:

- `AGENTS.md`
- `SECURITY.md`
- `docs/INDEX.md`
- `docs/architecture.md`
- `docs/rt-dispatch-architecture.md`
- `docs/timing-principles.md`
- `docs/distribution-and-update.md`
- `src/build_app.py`
- `.github/workflows/release.yml`
- `src/sky_music/domain/update_checker.py`
- `src/sky_music/orchestration/update_service.py`
- `src/sky_music/infrastructure/hotkeys.py`
- `src/sky_music/platform/win32/window_target.py`
- `Sky-Auto-Player.spec`
- `rust/Cargo.toml`

## Capture baseline

```powershell
git rev-parse HEAD
git status --short
uv run ruff check .
uv run pyright
uv run pytest
uv run --env-file .env python scripts/audit_security_mandates.py
uv run --env-file .env python scripts/audit_free_threaded_wheels.py
cargo fmt --manifest-path rust/Cargo.toml --all -- --check
cargo check --manifest-path rust/Cargo.toml --workspace --all-targets --all-features
cargo clippy --manifest-path rust/Cargo.toml --workspace --all-targets --all-features -- -D warnings
cargo test --manifest-path rust/Cargo.toml --workspace --all-features
```

Record active occurrences of:

```text
updater.bat
updater.ps1
ExecutionPolicy
Sky-Player.exe
GetAsyncKeyState
AttachThreadInput
SetWindowPos
BringWindowToTop
SetActiveWindow
collect_all(
```

## Normative docs

Change repository contract to:

- in-app check + native out-of-process apply;
- one release ZIP;
- Rust native updater;
- transactional update;
- preserve `config.json`, `.env`, `songs/`, `logs/`;
- no script updater;
- no old executable-name support;
- signing before manifest;
- app never mutates its own running payload;
- temp updater runner performs swap.

Update `AGENTS.md` and `docs/distribution-and-update.md` first, plus other normative docs as needed.

### DoD

- P0 unchanged.
- No normative doc still declares PowerShell updater as target architecture.
- Baseline tests green.

---

# Phase 1 — Hotkey and focus behavioral hardening

Follow `05_HOTKEY_FOCUS_HARDENING.md`.

Implement:

1. `RegisterHotKey` backend under `platform/win32`.
2. Message-loop thread + action queue.
3. Remove continuous playback hotkey `GetAsyncKeyState` polling.
4. Keep Rust physical-state safety checks.
5. Split window discovery from focus mutation.
6. Remove automatic aggressive focus reclaim.
7. Keep only minimal explicit user-requested refocus.
8. Remove unused ctypes declarations.

### DoD

No continuous Python hotkey polling.

Automatic focus path no longer relies on:

```text
AttachThreadInput
SetWindowPos(HWND_TOP)
BringWindowToTop
SetActiveWindow
```

unless a remaining use is independently justified and approved.

---

# Phase 2 — Native updater crate foundation

Add:

```text
rust/crates/sky_updater/
```

Register as fourth Rust workspace member.

Implement pure/testable components first:

- CLI validation;
- typed errors;
- strict asset naming;
- strict SHA sidecar parser;
- safe relative-path validator;
- manifest schema/parser;
- case-insensitive collision detector;
- preserve-path classifier;
- transaction plan builder;
- old/new manifest orphan calculation;
- result model.

Prefer stdlib + focused dependencies. No playback crate dependency.

### DoD

- crate builds;
- pure unit tests pass;
- no install mutation exists before verification/transaction preparation.

---

# Phase 3 — Complete updater network, verification and transaction engine

Follow `02_TARGET_ARCHITECTURE_AND_UPDATER_SPEC.md`.

Implement:

- parent process wait;
- exact release fetch by tag;
- WinHTTP HTTPS + redirect allow-list;
- bounded downloads;
- SHA sidecar;
- safe ZIP validation;
- staging;
- exact manifest verification;
- Authenticode verification plumbing;
- durable journal;
- backup;
- managed replacement;
- safe orphan cleanup;
- post-copy verification;
- rollback;
- interrupted-transaction recovery;
- result file;
- restart;
- true dry-run.

### DoD

- no mutation before `prepared` journal;
- failure after `prepared` fully rolls back or leaves durable fail-closed recovery material;
- preserved paths untouched;
- unknown unmanifested user files untouched.

---

# Phase 4 — App integration and Update now

## Update checking

Keep current Python stable/beta user-facing selection.

Harden apply readiness so exact expected assets are required:

```text
Sky-Auto-Player-v<version>.zip
Sky-Auto-Player-v<version>.zip.sha256
```

Do not use generic "first ZIP" for apply readiness.

## Add launcher

Recommended file:

```text
src/sky_music/infrastructure/update_launcher.py
```

Responsibilities:

1. validate install root/updater path;
2. create `%LOCALAPPDATA%\Sky-Auto-Player\update-runs\<uuid>`;
3. copy updater;
4. verify copy hash equals source hash;
5. launch updater without shell;
6. pass install root/PID/current version/target version/channel/restart;
7. report launch success/failure;
8. never install files itself.

No `ctypes` in infrastructure.

## Playback shutdown invariant

Before launching updater:

- if playback active, request normal graceful stop;
- mandatory key-up / panic-release cleanup must complete;
- updater launches only after cleanup;
- app exits only after updater launch succeeds.

Do not shortcut Rust cleanup.

## UI

Modal actions:

```text
Update now
Later
Skip this version
```

Remove instructions to run `updater.bat`.

If updater launch fails, app remains open and shows recoverable error.

## Startup result

On next startup:

- read `last-result.json`;
- show success/rollback/failure;
- mark/clear consumed result safely;
- cleanup stale update-run dirs conservatively.

### DoD

End-to-end dev update launch works without BAT/PowerShell apply path.

---

# Phase 5 — Clean removal of old updater architecture

Delete:

```text
updater.bat
installer/updater.ps1
installer/Tests/**
```

Delete updater-only actions after verifying no other workflow consumes them:

```text
.github/actions/updater-ps51-gate/**
.github/actions/run-pester/**
```

From `src/build_app.py` remove:

- `REQUIRED_UPDATER_ASSETS`;
- copy of `updater.bat`;
- copy of `installer/`;
- build-time PowerShell `ExecutionPolicy Bypass` helper if it remains.

Preferred locked-build behavior: fail clearly and ask developer to close stale process rather than auto-kill through PowerShell.

Remove old-name compatibility whose only purpose is update migration:

```text
Sky-Player.exe
dual-name updater resolution
bridge ZIP logic
```

Do not remove unrelated Sky game process-name logic without verifying context.

Update README/site/FAQ/troubleshooting/docs.

### DoD

No active update/distribution dependency on:

```text
updater.bat
installer/updater.ps1
ExecutionPolicy Bypass
Sky-Player.exe
```

Historical archived docs may mention them only when clearly historical.

---

# Phase 6 — Signing, manifest order and release workflow

Follow `04_BUILD_SIGNING_PACKAGING.md`.

Refactor production flow to:

```text
build
→ stage
→ smoke
→ sign own PE
→ verify signatures
→ generate manifest from signed bytes
→ verify manifest
→ ZIP
→ SHA256
→ E2E update
→ attest
→ publish
```

Sign at minimum:

```text
Sky-Auto-Player.exe
Sky-Auto-Player-Updater.exe
native_calibration.exe
_internal/**/sky_player_rs*.pyd
```

Do not re-sign third-party runtime DLLs.

Production tag must fail closed until real signing provider is configured.

### DoD

Manifest hashes signed bytes, not pre-sign bytes.

---

# Phase 7 — PyInstaller hardening

## 7A source-built bootloader

- resolve exact PyInstaller version from lock/build env;
- pin exact version for release toolchain;
- build matching upstream bootloader source;
- no bootloader behavior modifications;
- no packing/obfuscation;
- record compiler/version/hash;
- production packaging fails if source bootloader build fails.

## 7B reduce collection surface

Do separately from bootloader integration.

Reduce broad `collect_all(...)` one package at a time.

For each change:

1. inspect dynamic imports/data;
2. change one target;
3. frozen build;
4. selftests;
5. UI startup;
6. record tree/file-count/size delta.

Do not add aggressive `excludes` without proving non-use.

Keep `upx=False`.

---

# Phase 8 — Final qualification

Follow `06_TEST_VALIDATION_MATRIX.md`.

Complete:

- Python gates;
- Rust gates;
- updater fault tests;
- packaged update E2E;
- rollback E2E;
- signing verification;
- manifest exact-set verification;
- clean package assertion;
- Defender scan evidence.

No production release until mandatory gates are green.
