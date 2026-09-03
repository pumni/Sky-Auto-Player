# System Architecture

The Tauri/React desktop is the only supported user-facing product. Its
commands are handled by the native Rust composition root; the repository has
no tracked Python runtime or tooling surface.

## Layers

```text
React / TypeScript
        |
        v
sky_desktop_shell
        |
        +--> sky_app_core          pure application/domain policies
        +--> sky_native_adapters   filesystem, Win32 and process adapters
        +--> sky_player            playback application/runtime service
        +--> Tauri updater plugin  signed update check/download/install
        +--> sky_updater           retained legacy updater boundary
```

`sky_app_core` remains inward and must not depend on Tauri, Win32, the desktop
shell, `sky_player`, or concrete filesystem/network/process implementations.
Effects are composed by the shell and outer adapter crates. The only physical
keyboard input boundary is Windows `SendInput`; the application does not read
or modify another process, inject code, install hooks, or bypass anti-cheat.

## Native desktop ownership

All 21 stable desktop commands have one native handler. There is no Python
Core process, desktop Python IPC, or automatic fallback in the production
router. `core.ready` remains a compatibility wire event name and is produced
by the native runtime where the frontend contract requires it.

The native runtime owns settings persistence, catalog indexing/search/detail,
playback planning and control, diagnostics, update policy, calibration
orchestration, event delivery, and shutdown. The calibration measurement
remains process-isolated in `native_calibration.exe`. V4 update operations are
owned by the Rust `UpdateService`, which invokes the official Tauri updater;
`sky_updater` remains buildable for the legacy line until its retirement work
order.

## Playback boundary

```text
song file -> pure Rust parser/planner -> native playback adapter
          -> sky_player -> dispatch_core/dispatch_win32 -> SendInput
```

The qualified realtime worker remains authoritative for QPC scheduling,
MMCSS/priority, focus admission, supervisor lease, bounded queues, no-allocation
hot-path behavior, and emergency release-all cleanup. Supervisor controls and
diagnostics stay outside the realtime data path. Dry-run playback never reaches
the physical input backend.

## Calibration and update boundaries

Calibration is coordinated by the native application and measures through the
out-of-process Rust calibration executable. Evidence is bounded, validated for
schema, host, protocol, provenance, buckets, quantiles, and timing envelope
before atomic cache publication. Prepared playback is invalidated after an
admitted successful publication.

The Rust `UpdateService` owns stable/beta authority selection, bounded update
state/progress, playback admission, and the Tauri updater lifecycle. React
never receives endpoint, key, artifact, or downgrade policy data. Production
authority is fail-closed until WO-04 configures it; the signed packaged
previous-v4 to candidate-v4 fixture is test-only. The retained
`sky_updater` boundary continues to own the legacy line's transaction and
recovery behavior.

## Evidence and repository tooling

Production parity is proven by direct Rust/TypeScript tests and committed
fixtures. Historical words and frozen fixture data may remain as evidence, but
they are not executable tooling. The canonical release/check tooling is
`cargo xtask` and Bun.

Use the Rust repository tool for current verification groups:

```powershell
cargo xtask check static
cargo xtask check rust
cargo xtask check desktop
cargo xtask check all
```

The `xtask` retirement, architecture, security, branding, and zero-Python checks are canonical.
