# Changelog

All notable changes to Sky Auto Player are documented here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [3.1.0] - 2026-08-08

### Added

- Frame-based note-hold controls for clearer FPS-aware playback tuning.

### Changed

- Improved latency calibration and consistent use of calibrated timing margins across playback flows.
- Improved native scheduling for chords and timestamp-aligned input transitions.
- Reduced real-time dispatch overhead for steadier playback, especially during longer sessions.

### Fixed

- Hardened focus handling, input-state recovery, and cleanup around interrupted or failed input delivery.
- Improved playback HUD consistency and calibration guidance.

## [3.0.0] - 2026-08-03

### Changed

- Rust is the sole production dispatch implementation; Python retains UI, configuration, authored-action preparation, and application orchestration only.
- The deadline-to-`SendInput` path no longer calls Python.
- Removed the Python dispatch core, Python sender stack, low-level Rust input adapter, backend selector, runtime fallback, and migration environment flags.
- The PyO3 surface is reduced to `SessionConfig`, `DispatchSession`, startup admission metadata, and native calibration.
- Timing remains in the QPC tick domain until the serialization boundary.
- Native telemetry schema remains `6`; calibration final artifact schema is `5`.

### Added

- `sender_clean_known` distinguishes complete sender evidence from a clean result.
- Release CI and tagged release workflows run the native sender-side acceptance benchmark before packaging.
- V3 migration and acceptance documentation for the frozen Windows application and updater.

### Fixed

- Authored completion residual is separate from effective completion residual.
- Focus-blocked diagnostics are no longer counted as backend dispatches.
- Suppressed stale Up events no longer distort dispatch counts.
- Partial native sends are no longer classified as no-op events.
- Quick calibration validates the exact requested configuration.
- Full calibration rejects an impossible budget before side effects.
- Fabricated host timestamps are no longer emitted.
- Truncated telemetry can no longer report clean evidence: `sender_clean_known` and `sender_clean` are both false.

### Performance

- Dedicated Rust dispatch worker with preallocated bounded telemetry.
- No Python callback runs on the real-time worker.
- Native sender-side acceptance and regression benchmark is a release gate.

### Compatibility

- Supported packaged platform: Windows 10/11 x64.
- Source development uses CPython 3.14 free-threaded; packaged releases include their own runtime.
- Existing `2.4.5` installations update directly through `updater.bat`.
- Pre-2.4.2 installations that never ran the rename bridge still require manual reinstall.
- `config.json`, `.env`, `songs/`, and `logs/` are preserved during updates.

### Migration

- Native telemetry schema 5 artifacts are rejected by the schema 6 consumer; old native telemetry is diagnostic history, not V3 release evidence.
- Calibration schema 4 checkpoints are rejected by the schema 5 loader; run calibration again for V3.
- The legacy latency cache remains accepted only when it matches loader contract version 1 and its strict sample/value bounds; otherwise the loader uses its default fallback.
- Rollback is performed by rolling back the application release; the active binary contains no Python dispatch engine.

### Known limitations

- `game_observed.available = false`; telemetry stops at the sender completion boundary.
- This release does not claim that the game receives input at the same time, absolute audio onset, universal sub-millisecond timing, or zero jitter on every machine.

## [2.4.5] - 2026-07-29

### Removed

- **Retired the dual-publish legacy bridge.** Releases from this version on ship only the canonical triple (`Sky-Auto-Player-v<ver>.zip` + `.sha256` + `MANIFEST.json`); the legacy `Sky-Player-v<ver>.zip` bridge pair is no longer built or uploaded. The bridge (introduced in 2.4.2 to migrate pre-rename installs via the old `updater.bat`) was the sunset target of plan D3.
- Removed the `build_legacy_bridge_dir` builder and its unconditional `--manifest` call site in `src/build_app.py`.
- Removed the bridge stage / assert / attest / upload steps from `.github/workflows/release.yml`.

### Changed

- `installer/updater.ps1` `Select-ReleaseAssets` no longer falls back to a `Sky-Player-v<ver>.zip` asset pair; release ingest is canonical-only. `Resolve-PrimaryExe`, `Resolve-ProcessNames`, `updater.bat`, and `Resolve-StagingRoot` keep accepting a pre-existing `Sky-Player.exe` so already-bridged installs continue to update cleanly (D11 orphan-cleanup path).

### Compatibility

- **Pre-2.4.2 installs that never ran the bridge are stranded.** They must reinstall manually from the canonical `Sky-Auto-Player-v2.4.5.zip`, preserving `config.json`, `.env`, `songs/`, and `logs/`. Already-bridged installs (v2.4.2 → v2.4.4) keep updating transparently.

### Fixed

- Orchestration: the wait-spin-start offset now uses `effective_spin_threshold` so the cold-core warmup budget is reflected in `pre_send_spin_us` telemetry on the cold path.
- Orchestration: HUD onset counters use `visible_lateness_us` (player-perceived onset) instead of call-entry `lateness_us`; release counters keep the bounded-retry metric.
- Orchestration: `DirectProgressSink.publish()` forwards counters via `update_counters_batch`, restoring observability parity with the threaded snapshot sink.
- Scheduler: the pre-play and mid-song spin probes use the worst observed wake error plus 200 µs instead of p90, blocking rare timer-coalescing spikes from leaking through the spin guard.
- Build: `Sky-Auto-Player.spec` `optimize=1` comment corrected (Python strips `assert` at `optimize>=1`); a `--selftest-optimize` smoke step now fails the build if the frozen binary regresses to `__debug__ == True`.

### Performance

- Orchestration: `pop_due_pending` takes a single-key fast path for the dominant one-pending release case, skipping list comprehension / sort / tuple allocation.
- Windows backend: `SCAN_TO_VK` inverse map cached at module load from `sky_music.layouts`; removes ~15 µs of per-call work from the abort path (`release_all` / panic pause / focus loss / quit / finished).

### Refactor

- Domain layer is purer: `SleepPolicy` construction moves from `domain/session_context` to orchestration / CLI / UI callers, and the calibration loader moves from `domain/scheduler_types` to `infrastructure/calibration_loader`. The domain now returns primitives; orchestration materialises the infrastructure shape.
- Dead code dropped: unused `DispatchLoop.enable_event_wait` instance field; redundant `Queue.empty()` pre-check in `QueueCommandSource.poll` (few hundred ns of avoided jitter per control poll under no-GIL).
- Tooling: `scripts/measure_dispatch_tail.py` rewritten for 3.14t (fail-fast under GIL-on / old interpreter, fixed seed for reproducible p50/p99, the four 3.14t axes replacing the GIL switch-interval matrix); a new `tests/bench_dispatch_send_pedantic.py` hot-path microbench pins the p50 ≥ 10% / p99 ≤ 5% gate future candidates must beat.

## [2.4.4] - 2026-07-25

### Fixed

- Made the Windows PowerShell 5.1 release compatibility gate ASCII-safe and explicit about the updater BOM.

## [2.4.3] - 2026-07-25

### Fixed

- Hardened the external updater's launch-path detection and TEMP staging path
  handling, including Windows 8.3 short-name paths.
- Expanded updater regression coverage for manifest verification, rollback,
  preserve-list safety, release asset selection, and legacy migration.

### Compatibility

- The Sky-Player legacy bridge remains published alongside the canonical
  Sky-Auto-Player package during the migration window.

## [2.4.2] - 2026-07-24

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

> Users running a pre-rename build (v2.4.1 or earlier) can seamlessly migrate 
> using their existing `updater.bat` once v2.4.2 is published. This release 
> publishes a temporary legacy bridge zip alongside the canonical one, which 
> allows the old updater to download the new brand and the new updater 
> scripts in one shot, preserving `config.json` and `songs/` unchanged.

### Added

- **Dual-Publish Bridge:** Added support for publishing a legacy bridge zip (`Sky-Player-v<ver>.zip`) alongside the canonical `Sky-Auto-Player-v<ver>.zip` to support seamless updater migrations from old versions.
- **Dual-Name Updater:** `updater.bat` and `installer/updater.ps1` now support both `Sky-Auto-Player.exe` and `Sky-Player.exe` for process guard and identity resolution.

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
