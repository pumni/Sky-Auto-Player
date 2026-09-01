# Rust Architecture

## Crate dependency direction

The production graph is direct Rust:

```text
sky_desktop_shell
       |
       +--> sky_app_core
       +--> sky_native_adapters
       +--> sky_player
       +--> sky_updater

sky_player -> sky_dispatch_core + sky_dispatch_win32
```

`sky_app_core` is pure application/domain code. It must not depend on Tauri,
Win32, the desktop shell, `sky_player`, or concrete outer adapters. The shell
and outer adapters compose effects around those inward policies.

## Module ownership

Each state has one owner and each module has a focused invariant. The
`sky_player` engine owns session lifecycle, worker admission/control,
QPC-based timing, target/focus gates, telemetry publication, and cleanup.
`sky_dispatch_core` owns pure schedule/action policy; `sky_dispatch_win32`
owns the Windows wait, focus, priority, and `SendInput` boundaries.

The desktop shell owns Tauri command decoding and delivery DTO translation.
It does not contain song scheduling, risk analysis, persisted-settings
migration, or realtime dispatch algorithms. `sky_updater` owns network,
manifest, transaction, rollback, and recovery implementation at its existing
security boundary. Calibration measurement is a separate native process.

## Hot-path constraints

The healthy Down/Up path must not introduce heap allocation, locks, extra timing
queries, dynamic dispatch, blocking/unbounded communication, or reference-count
clones. The realtime worker keeps bounded/nonblocking queues and emergency
release-all cleanup. Any realtime change requires the specialized noalloc,
assembly, and Windows timing evidence.

## Unsafe boundary

Unsafe code is restricted to the approved Win32 input and wait modules and
must carry a `// SAFETY:` explanation. Application orchestration, Tauri
delivery, calibration validation, and updater policy remain safe Rust.

## Test support and checks

Test seams are feature/test gated and are never selected by an environment
variable alone. Production package qualification composes safe seams
explicitly while the normal runtime uses disabled seams. Run:

```powershell
cargo xtask check static
```

There is no foreign-language extension crate, wheel build, or Python player
binding in the Rust workspace.
