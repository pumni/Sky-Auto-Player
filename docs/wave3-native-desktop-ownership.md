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

The labels below are intentionally conservative:

* **PROVEN PARITY** means a committed Python-oracle or direct contract test
  covers the stated behavior.
* **NATIVE IMPLEMENTATION** means Rust owns the executable path, without
  implying complete cross-runtime parity for every related behavior.
* **TRANSITIONAL CROSS-OWNER BRIDGE** means Python remains authoritative while
  the shell performs explicit synchronization or event relay.
* **EXPLICITLY UNMIGRATED** means the behavior is intentionally still Python
  or legacy owned.

### Proven parity

* Persisted settings normalization and migration are exercised through the
  real `JsonSettingsStore -> SettingsService` path and committed Python
  fixtures. Live settings commands remain Python-owned; the shell invalidates
  native prepared plans under a shared coherence lock after a successful
  Python patch.
* Catalog IDs, canonical path handling, generation checks, Unicode NFKD/mark
  removal/`đ` replacement/casefold normalization, stable ordering, substring
  behavior, bounds, and the committed WRatio reference corpus are tested in
  pure Rust. The native catalog uses the bounded WRatio-compatible ranker;
  rich detail and live authority are separately called out below.
* The committed corpus covers the current Python song parser, scheduler, risk,
  calibrated-policy, and canonical fingerprint cases exercised by this slice.
  It is generated from the current implementation and immutable in normal CI;
  the full playback state-machine and focus-loss trace corpus remains a
  native implementation/qualification surface, not a blanket parity claim.
* Update channel and retry policy behavior is fixture-backed.

### Native implementation

* The native runtime owns the 13 commands listed above. Handler completeness
  is checked against the dispatch branches, not only the policy matrix.
* `UiEventHub` is bounded, coalesces latest-wins snapshot traffic, preserves
  lifecycle events, validates payload bounds, and fails closed on lifecycle
  overflow. Native events and decoded Core events use the same Tauri delivery
  channel; per-source order is preserved, while no unproven global
  cross-source order is promised.
* Native playback uses the direct Rust player boundary. Ordinary tests use
  dry-run or safe seams; the direct supervisor retains the qualified
  3,000,000 µs lease, 20/200/100 ms control/focus/heartbeat/snapshot cadence,
  exact-HWND verification, calibrated timing policy, and physical-input
  worker final gate. The realtime worker itself remains unchanged. Full
  prepare/control/shutdown trace parity remains an explicit Native
  implementation qualification surface.
* Update policy and preferences remain Python-owned as one coherent family
  until the hardened gateway boundary and Core cache can move together.

The shell-level `ActivityCoordinator` is the single non-realtime gate for
Native physical playback and Python calibration. It is atomic in either request
ordering and is released on terminal, failure, cancellation, Core-failure,
and shutdown paths. Successful calibration events also invalidate native
prepared plans before being delivered.

### Transitional cross-owner bridges

* `settings.get` and `settings.patch` remain Python-owned because Core retains
  a process-local settings cache. Successful Python settings patches are
  serialized with native playback start and invalidate native prepared state.
* `calibration.start` and `calibration.cancel` remain Python-owned. Their
  admission is coordinated with native physical playback and Core terminal
  events are relayed through the native event hub.

### Explicitly unmigrated

* Catalog WRatio/index/detail behavior is native only where the corresponding
  contract is implemented; the Python catalog remains the oracle and no
  independent Python generation is used for the native route.
* Update network orchestration, exact handoff, and all calibration process /
  evidence-validation / cancellation behavior remain Python-owned.
* The remaining eight Python command routes are the complete update family,
  complete settings family, and calibration start/cancel as listed above.
* Python rich catalog authority, any behavior not covered by the committed
  native substring/WRatio fixtures, and the PyO3 wheel remain rollback/oracle
  surfaces. Python source, Textual, and `sky_player_rs` are retained and are
  not deleted or declared obsolete by Wave 3.

## Packaging status

The existing portable release still includes `Sky-Auto-Player-Core.exe` and
its Python `_internal` runtime because eight desktop command methods still
depend on Core. Sidecar retirement is not claimed. It is safe to remove those
files only after the remaining Python-owned routes, event/startup/shutdown
retirement gate, and full cross-owner parity are independently proven.
