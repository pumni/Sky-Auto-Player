# Wave 4 — final native desktop ownership

Wave 4 starts from `main@cbbdff15e831ce2eee65d4af6258c5601ca00730` and moves the
remaining desktop command authority into the native Tauri composition root. The
integration branch is `wave4/final-native-ownership-integration`; this document is
evidence for that branch and must not be read as merge approval.

## Evidence vocabulary

- **PROVEN PARITY** — committed Python-oracle fixtures or an independently specified
  contract pass through the production route under test.
- **NATIVE IMPLEMENTATION** — the production Tauri route is implemented by Rust and
  its effects remain behind outer adapters.
- **RETIRED PRODUCTION PATH** — the old path is absent from the canonical portable
  artifact and normal Tauri startup.
- **RETAINED ORACLE/ROLLBACK** — repository source, tests, or build support remains
  intentionally available without being a production runtime dependency.

## Ownership

The final delivery matrix is 21 Native / 0 Python. Every stable desktop command has
one executable `NativeDesktopRuntime::dispatch` handler and the router has no Python
fallback branch. The frontend command names, request/response DTOs, generated
bindings, and `UiEvent` wire shape remain unchanged.

The native composition root owns Settings, Catalog, Playback, Diagnostics, Update,
Calibration, bootstrap, event delivery, and shutdown. `sky_app_core` remains inward;
filesystem, process, Win32, updater, and physical calibration effects remain in outer
adapters or isolated native child processes.

## Settings and update

**PROVEN PARITY:** normalized persisted Settings, v2/v3 migration, coercion,
unknown-field preservation, atomic patch behavior, update preferences, and update
policy are covered by frozen oracle fixtures and production-route tests.

**NATIVE IMPLEMENTATION:** one live native settings store is shared by bootstrap,
catalog detail, playback preparation, update policy, and calibration policy. A
successful settings mutation invalidates prepared playback before a later start can
consume it. Update metadata selection and handoff use `sky_updater`'s hardened
manifest, hash, HTTPS, transaction, rollback, and recovery boundaries. The Native
handoff also reserves its application-level `HandoffStarting` state before any
run-directory or child-process side effect, rejects redirected update state roots,
and performs the install-root durable-write preflight before staging.

## Calibration

**NATIVE IMPLEMENTATION:** the GUI owns calibration orchestration and launches the
qualified `native_calibration.exe` child; physical measurement remains process
isolated. Rust validates bounded output and evidence before atomically publishing a
cache, then invalidates prepared playback and emits the terminal result. Activity
coordination prevents calibration and physical playback from running concurrently.
Calibration budgets preserve the established publication/process-exit/cleanup
reserves, and an out-of-envelope result is written with a null applied margin so it
round-trips through the accepted loader as a fallback rather than as a fake device
margin.

The final cache replacement, prepared-plan invalidation, and terminal-event
publication share one closing admission gate with calibration shutdown. Shutdown
closes that gate before waiting for workers; an admitted commit is completed within
the bounded shutdown contract, while an unadmitted worker can only discard its
temporary cache. If shutdown has already begun, the admitted cache/invalidation
outcome is retained and its terminal event is suppressed; shutdown owns the final
bounded lifecycle boundary. Activity admission is typed: a duplicate calibration reports
`already_running`, physical playback reports `playback_active`, and concurrent
playback/calibration requests have one shared linearization point.

**SAFE SELFTEST COMPOSITION:** package qualification enters `TestSeams::SafePackage`
only through the hidden selftest composition roots. The normal `run()` composition
always uses `TestSeams::Disabled`; environment variables alone cannot synthesize
calibration evidence or turn an update check into a false successful no-op. The
safe seam is therefore a qualification composition, not a production runtime
switch.

## Core retirement

**RETIRED PRODUCTION PATH:** the canonical Tauri path does not construct
`CoreSupervisor`, launch `Sky-Auto-Player-Core.exe`, use Python desktop IPC, or depend
on a Python interpreter. The historical `core.ready` event name is still emitted once
by the native runtime for frontend compatibility.

**RETAINED ORACLE/ROLLBACK:** Python source and tests, legacy Textual/CLI code,
PyO3/maturin compatibility code, `sky_player_rs`, and PyInstaller specifications
remain in the repository. They are not part of the canonical runtime-zero portable
artifact.

## Portable artifact

The canonical package contains the native Tauri executable, updater, isolated native
calibration executable, manifest, frontend assets, and required user-facing assets.
It rejects `Sky-Auto-Player-Core.exe`, `_internal/`, CPython DLL/ZIP files,
`base_library.zip`, PyInstaller runtime files, and Python extension modules. The
updater migration tests cover removal of managed sidecar files while preserving user
configuration, songs, and rollback behavior.

The Python runtime boundary is therefore **zero for the production portable desktop**;
the exact package also runs its Native selftest and GUI smoke with a restricted PATH
that cannot discover repository or system Python. This does not claim that the
repository itself is Python-free.
