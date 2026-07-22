# Changelog

All notable changes to Sky Auto Player are documented here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed — branding

- **Renamed from "Sky Player" to "Sky Auto Player".** The executable, build
  artifacts, release zip names, external updater URLs, in-app update checker,
  log directory, and user-facing strings all switched from
  `Sky-Player` / `sky-player` to `Sky-Auto-Player` / `sky-auto-player` to
  match the renamed GitHub repository (`pumni/Sky-Auto-Player`).
  - Executable: `Sky-Player.exe` -> `Sky-Auto-Player.exe`.
  - Release zip: `Sky-Player-v<ver>.zip` -> `Sky-Auto-Player-v<ver>.zip`
    (and the `.sha256` sidecar accordingly).
  - External updater log dir: `%LOCALAPPDATA%\Sky-Player` ->
    `%LOCALAPPDATA%\Sky-Auto-Player` (a new file is created on next update;
    the old log is left in place untouched).
  - `pyproject.toml` project name: `sky-player` -> `sky-auto-player`.
  - In-app update checker default repo: `Sky-Player` -> `Sky-Auto-Player`
    (queries `api.github.com/repos/pumni/Sky-Auto-Player`).
  - `Sky-Player.spec` renamed to `Sky-Auto-Player.spec`.
  - JSON-LD `alternateName` on the landing site still includes the legacy
    names (`Sky-Player`, `Sky Player`) so existing search traffic finds the
    renamed project; the canonical `name` is now `Sky Auto Player`.

> Users running a pre-rename build who manually delete the old folder and
> download the new zip get a clean install. Updater-assisted migrations from
> a `Sky-Player.exe` install are not supported — the external updater now
> looks for `Sky-Auto-Player.exe` and will exit 1 from `updater.bat`. Treat
> the rename as a fresh install.

## [2.4.0] - 2026-07-18

### Changed — breaking

- **In-app auto-update is removed.** Sky Player now notifies you when a new version is
  available; applying it is done by running the new `updater.bat` in the install folder, then
  reopening `Sky-Player.exe`. This moves Sky Player to a portable-distribution model and removes
  in-place file-replacement logic from the running app. The previous "Auto-apply without
  asking" toggle is removed from Update Settings.
- "Check for Update" in the picker now surfaces a banner modal with three actions: Open
  Releases page, Skip this version, Dismiss. The Download-and-apply progress modal is removed.
- The `update.auto_apply` and `update.pending_update_version` fields in `config.json` are
  no longer read or written. Existing entries in older `config.json` files are ignored
  silently and stripped on next save.

### Added

- `updater.bat` (repo root) and `installer/updater.ps1` — external updater. Verifies SHA256
  before any install mutation; verifies directory write access; stages in TEMP; backs up and
  copies binaries transactionally with a fallback rollback routine on failure. Preserves
  `config.json` and skips the `songs/` folder completely to avoid modifying user data.
  Log: `%LOCALAPPDATA%\Sky-Player\updater.log`. Supports `-Channel stable|beta`, `-DryRun`,
  `-ForceClose`, `-Restart`.
- `update.channel` (default `stable`), `update.last_notified_version`, and (until 2.4.1)
  `update.legacy_old_dir_sweep_pending` in `config.json`. Channel is wired to in-app check
  (`include_prerelease`) and to the external updater.
- One-time sweep of legacy `.old.{guid}` install siblings left from pre-2.4.0 atomic swaps.
  Runs silently when migration keys are present or leftovers are detected; removed in a
  follow-on minor.
- `.github/workflows/release.yml` — release on tag `v*` with tag↔`pyproject.toml` version lock,
  free-threaded audit, attest-build-provenance, three assets.
- `docs/distribution-and-update.md` — contributor documentation.
- **CI workflow** at `.github/workflows/ci.yml` executes the full altitude table
  (`audit_free_threaded_wheels` → `ruff` → `pyright` → `audit_security_mandates`
  → `pytest`) on `windows-latest` against the free-threaded interpreter.
- **Pre-commit config** at `.pre-commit-config.yaml` mirrors the same gates
  locally and adds `check-yaml` / `check-toml` / `check-json` / `eol` /
  `trailing-whitespace` so formatting drift is caught before push.
- **Pytest markers** (`scheduler`, `windows`, `golden`, `slow`) and
  `norecursedirs` (`golden_schedules`, `perf-baselines`, `.tmp`, `.claude`)
  declared via `[tool.pytest.ini_options]`.
- **`.editorconfig`** pins UTF-8, LF, 4-space indent for Python and 2-space
  for YAML/TOML/JSON; CRLF preserved on `*.bat`.
- **`PULL_REQUEST_TEMPLATE.md`** + **issue templates** (`bug_report.md`,
  `feature_request.md`, `security_p0.md`, `config.yml`) so every PR carries the
  altitude-table checklist and every security finding follows the disclosure path.

### Removed

- `apply_update_and_restart`, `write_apply_batch`, `apply_staged_update`,
  `download_and_verify_update`, `download_and_apply_update_worker`, `_apply_staged` from the
  app and service layers.
- `UpdateProgressModal` from `src/sky_music/ui/textual_app/modals.py`.
- `find_old_backups`, `post_update_flag_path`, `write_apply_batch`, `apply_update_and_restart`
  from `src/sky_music/infrastructure/update_installer.py`. The following helpers remain:
  `download_zip`, `compute_sha256`, `verify_sha256`, `parse_sha256_sidecar`,
  `fetch_sha256_sidecar`, `extract_zip`, `stage_update`, `install_dir_for_frozen`.
- `simulate_update.py` scenarios `download-ok` and `download-bad-sha` (they exercised the
  removed download-and-verify path).
- **`use_ll_hook` machinery** — opt-in global `WH_KEYBOARD_LL` hook
  (`SetWindowsHookExW`), the dormant `src/sky_music/infrastructure/hotkey_hook.py`
  module, the `AppConfig.use_ll_hook` field, the `PlaybackControls._hook`
  slot, and the `use_ll_hook` reading in `main.build_playback_controls`.
  The hotkey mechanism now relies exclusively on the focus-gated poll path
  (`is_virtual_key_down`); this aligns the runtime with `AGENTS.md` P0.1
  ("NO GAME TAMPERING — no hooks") and removes the only outstanding entry
  from `.config/security_audit_baseline.json`.

### Security

- **AGENTS.md P0 audit enforced in CI.** New `scripts/audit_security_mandates.py`
  (AST scanner) runs on every push and PR alongside the existing
  `audit_free_threaded_wheels.py` precheck. The audit forbids `ReadProcessMemory`,
  `WriteProcessMemory`, `SetWindowsHookEx*`, `CreateRemoteThread`, `DebugActiveProcess`,
  `NtQueryInformationProcess`, imports of `pymem`/`pyinject`/`win32api`, and `WinDLL("ntdll.dll")`,
  while explicitly allowing only the `SendInput` family. Historical violations
  are tracked in `.config/security_audit_baseline.json`.
- **Public `SECURITY.md`** now restates the P0 mandates and the disclosure channel
  (`security@pumni.dev`) for vulnerability-grade findings.



## [2.3.4] - 2026-07-17

### Changed

- **Refactored the dispatch loop and playback engine** for cleaner focus handling
  and tighter spin-threshold management.
- **Reworked timer management** in the main loop and playback supervisor for
  improved accuracy and performance.
- **Isolated the dispatch core** behind a structural interface, decoupling it
  from platform backends so the scheduler stays pure and unit-testable.
- **Removed the deprecated alias** for the abort input method in `DispatchLoop`.
- **Added an update-flow simulator** (`simulate_update.py`) for exercising
  update scenarios without a live network.
- **Documented the UI CPU/RAM optimization plan** for the 2026-07 workstream.

### Performance

- **Phase 1–3 hot-path hardening**: telemetry flush, cheap focus gate,
  symmetric reprobe, and uncontaminated overshoot samples — lowering tail
  latency on the dispatch spin path.

### Fixed

- **Phase 1 correctness**: focus gate, pause owner, clock, and estimator
  adjusted to remove residual bias and timing drift.

### Housekeeping

- **Backstop the O(polyphony) memory hardening** of `RuntimeDispatchCoordinator`
  introduced in commit `26d9b00`. That fix reduced `status_by_generation` from
  O(note_count) to O(polyphony) (≤ ~30 live entries); this release adds the
  regression coverage the original fix lacked.

## [2.3.3] - 2026-07-15

### Added

- `tests/test_runtime_dispatch_bounded_memory.py` — regression tests asserting that
  `RuntimeDispatchCoordinator.status_by_generation` stays bounded by polyphony
  (≤ 2 × scan_code_space) regardless of song length.
- Hardening assertion in `RuntimeDispatchCoordinator.generation_status_counts()`
  against silent counter drift (terminal + non-terminal > generation_count).
- Direct-drive instrumentation in `scripts/mem_attrlite.py` and
  `scripts/mem_engine_attr.py`; the previous approach inspected
  `engine._runtime_coordinator` post-play, which is `None` after `play()` returns.
- `CHANGELOG.md`.
- Bidirectional-invariant docstring on `status_by_generation`.

### Housekeeping

- Backstop the O(polyphony) memory hardening of `RuntimeDispatchCoordinator`
  introduced in commit `26d9b00`. That fix reduced `status_by_generation` from
  O(note_count) to O(polyphony) (≤ ~30 live entries); this release adds the
  regression coverage the original fix lacked.
