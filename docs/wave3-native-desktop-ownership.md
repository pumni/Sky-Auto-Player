# Wave 3 native desktop ownership

Wave 3 is the native desktop strangler boundary on top of the accepted Wave 2
baseline. The Tauri command names, request/response DTOs, generated frontend
contract, and serialized `UiEvent` shape remain unchanged.

## Current ownership

The delivery matrix is explicit and has one owner per command:

```text
Native: 13
Python: 8
```

Native owns bootstrap/shutdown, catalog index/detail/reload/viewport, playback
prepare/start/stop/pause/resume/skip, and diagnostics enablement. Python still
owns the complete settings family, the complete update family, and calibration:
`settings.get`, `settings.patch`, `update.check`, `update.preferences.get`,
`update.preferences.patch`, `update.begin_handoff`, `calibration.start`, and
`calibration.cancel`.

Settings remain Python-owned deliberately: Core's process-local `AppConfig`
cache is still live. Native services read the same atomically persisted store
before native use, but no second Native write authority or Native-to-Python
fallback is introduced.

There is no production native-to-Python fallback. A failure is returned by the
selected owner. The Python Core process is therefore still required by the
normal GUI path and remains in the portable artifact during this wave.

## Composition and safety boundaries

```text
Tauri delivery
    -> NativeDesktopRuntime
        -> sky_app_core / sky_native_adapters
        -> sky_player -> dispatch crates
        -> sky_updater (hardened outer implementation)
    -> CoreSupervisor only for the eight explicit Python routes
```

`sky_app_core` remains inward and forbids Tauri, PyO3, Win32, the desktop
shell, player, and concrete adapters. Filesystem access stays in
`sky_native_adapters`; the production install root is derived from the running
executable's parent, not the process working directory.

The qualified `sky_player` realtime worker remains authoritative for physical
input. Wave 3 changes its non-realtime application/control-plane caller only;
it does not change QPC timing, MMCSS/priority behavior, SendInput, focus
admission, bounded queues, release-all cleanup, or the hard-path allocation
contract.

## Evidence status

### Proven parity

* Persisted settings normalization and migration are exercised through the
  real `JsonSettingsStore -> SettingsService` path and committed Python
  fixtures.
* Catalog IDs, canonical path handling, generation checks, Unicode NFKD/mark
  removal/`đ` replacement/casefold normalization, stable ordering, substring
  behavior, bounds, and the committed WRatio reference corpus are tested in
  pure Rust.
* Song parser, scheduler, and risk fixtures are generated from the current
  Python implementation and consumed by Rust tests. The corpus is immutable in
  normal CI.
* Update channel and retry policy behavior is fixture-backed.

### Native implementation

* The native runtime owns the 13 commands listed above.
* `UiEventHub` is bounded, coalesces latest-wins snapshot traffic, preserves
  lifecycle events, validates payload bounds, and fails closed on lifecycle
  overflow. Native events and the remaining Core event stream use the same
  Tauri delivery channel; per-source order is preserved, while no unproven
  global cross-source order is promised.
* Native playback uses the direct Rust player boundary. Ordinary tests use
  dry-run or safe seams; physical-input qualification remains in the existing
  native/player evidence paths.
* Update policy and preferences remain Python-owned as one coherent family
  until the hardened gateway boundary and Core cache can move together.

### Explicitly unmigrated

* RapidFuzz WRatio is represented by a bounded pure-Rust compatibility ranker
  and checked against the committed reference corpus, but rich catalog detail
  and any live Python catalog authority outside the native route are not
  removed from the repository.
* Update network orchestration, exact handoff, and all calibration process /
  evidence-validation / cancellation behavior remain Python-owned.
* Python source, Textual, PyO3/maturin, `sky_player_rs`, and Python tests are
  retained as oracle and rollback material. They are not deleted or declared
  obsolete by Wave 3.

## Packaging status

The existing portable release still includes `Sky-Auto-Player-Core.exe` and
its Python `_internal` runtime because eight desktop command methods still
depend on Core. Sidecar retirement is not claimed. It is safe to remove those
files only after all remaining update and calibration ownership has moved and
the native event/startup/shutdown retirement gate is independently proven.
