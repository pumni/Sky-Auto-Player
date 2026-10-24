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
```

`sky_app_core` remains inward and must not depend on Tauri, Win32, the desktop
shell, `sky_player`, or concrete filesystem/network/process implementations.
Effects are composed by the shell and outer adapter crates. The only physical
keyboard input boundary is Windows `SendInput`; the application does not read
or modify another process, inject code, install hooks, or bypass anti-cheat.

### Module ownership

Each state has one owner and each module has a focused invariant:

- `sky_app_core` owns pure application and domain policy (parsing, validation, sheet models).
- `sky_native_adapters` encapsulates concrete filesystem, process, and Win32 host adapters.
- `sky_player` owns session lifecycle, worker admission/control, QPC-based timing, target/focus gates, telemetry publication, and cleanup.
- `sky_dispatch_core` owns pure schedule and action policy.
- `sky_dispatch_win32` owns Windows wait, focus, priority, and `SendInput` boundaries.
- `sky_desktop_shell` (`desktop/src-tauri`) owns Tauri command decoding, delivery DTO translation, and the Rust-owned `UpdateService` policy boundary. It contains no song scheduling, risk analysis, persisted-settings migration, or realtime dispatch algorithms.

## Native desktop ownership

All 32 stable request/response commands have one native handler. The separate
`subscribe_ui_events` Tauri command establishes the UI event subscription; it
is not a request/response method in the IPC method table. There is no Python
Core process, desktop Python IPC, or automatic fallback in the production
router. `core.ready` remains a compatibility wire event name and is produced
by the native runtime where the frontend contract requires it.

`NativeDesktopRuntime` in `desktop/src-tauri/src/native_runtime.rs` remains the
composition and ownership root for settings, command routing, app coordination,
the shared `NativeEventHub`, and shutdown. Its child modules own focused
control-plane work while using the same runtime state:

- `native_runtime/catalog.rs` owns catalog and library operations, including
  generation-fenced loading, reconciliation, cache handling, imports, search,
  and detail projection.
- `native_runtime/calibration.rs` owns calibration orchestration, worker
  cancellation, evidence validation, and admitted atomic cache publication.
  Measurement remains process-isolated in `native_calibration.exe`.
- `native_runtime/playback/` owns playback planning and control through
  `NativePlaybackService`; the qualified worker and sender boundaries are
  unchanged.
- `NativeEventHub` stays in `native_runtime.rs` and owns shared catalog,
  calibration, and playback event delivery. Its subscription, queue,
  publisher, and fail-closed behavior are unchanged.

V4 update operations remain owned by the Rust `UpdateService`, which invokes
the official Tauri updater. The native test suites are grouped under
`native_runtime/tests/` by catalog, calibration, event delivery, and playback
integration.

The architecture audit applies explicit size budgets to the native composition
root, its ownership modules, and their grouped test suites. Remaining module
size debt in `rust/xtask/src/checks.rs` and
`rust/crates/sky_player/src/engine/tests.rs` is outside this maintenance scope.

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

### Physical target owner authority

A physical require-focus session binds the selected HWND to its startup owner
PID, process creation time, and full image path. The session retains one process
handle opened with `PROCESS_QUERY_LIMITED_INFORMATION`; the desktop supervisor
checks that process object and the HWND owner off the realtime thread. A target
generation change invalidates the binding. Rebinding requires fresh
control-plane validation and a new generation, including when Windows reuses
the same numeric HWND.

Before each Down-bearing send, the worker samples the foreground HWND once and
queries the owner PID once for that exact HWND. It rejects a mismatch or failed
query before the sender. UpOnly safety release and sessions with focus checks
disabled do not query the foreground owner.

If the supervisor observes process termination, HWND destruction, owner drift,
or identity-query failure, it invalidates Down authority and requests terminal
cleanup. A same-process window destroyed and recreated with the same numeric
HWND entirely between supervisor checks can remain indistinguishable: the PID,
creation time, image, and final owner query can all still match. The HWND APIs
do not expose a stable window-lifetime identifier for that case, and the final
owner check retains the existing query-to-`SendInput` race.

## Calibration and update boundaries

Calibration is coordinated by the native application and measures through the
out-of-process Rust calibration executable. Evidence is bounded, validated for
schema, host, protocol, provenance, buckets, quantiles, and timing envelope
before atomic cache publication. Prepared playback is invalidated after an
admitted successful publication.

The Rust `UpdateService` owns stable/beta authority selection, bounded update
state/progress, playback admission, and the Tauri updater lifecycle. React
never receives endpoint, key, artifact, or downgrade policy data. Production
authority is fixed to the dedicated v4 stable/beta metadata paths; missing
metadata or missing/invalid v4 updater trust material fails closed. Production
Authenticode uses the governed `unsigned-zero-budget` policy; no provider
credentials are required, and qualification rejects signed or unexpected PE state.
The packaged previous-v4 to candidate-v4 fixture is test-only and consumes the
exact installer/signature bytes re-downloaded from a release draft. The retired
v3 transaction/recovery implementation is preserved in Git history and the
`v3-maintenance` line; it is not part of the current workspace or product graph.

## Unsafe boundary and test seams

Production unsafe Rust is limited to the reviewed platform boundaries below.
The newly audited desktop and native-adapter blocks carry local `// SAFETY:`
justifications; the existing Win32 adapter modules remain separately listed in
the architecture allowlist. Application orchestration, domain policies
(`sky_app_core`), Tauri command delivery, catalog indexing, calibration
validation, and updater policy remain safe Rust.

| Module | Unsafe operation | Safety invariant |
| --- | --- | --- |
| `rust/crates/sky_dispatch_win32/src/{calibration,clock,cpu,event,focus,focus/window_identity,input/packet,input/physical,input/raw,mmcss,power,timer,wait/hybrid}.rs` | Win32 handles, callbacks, timing, packet construction, and `SendInput` | Platform calls stay in the Windows adapter; owned handles are closed by their owner and physical input uses `SendInput` only. |
| `rust/crates/sky_native_adapters/src/lib.rs` | `MoveFileExW` atomic settings replacement | NUL-terminated source and destination buffers stay live through the synchronous replace call. |
| `desktop/src-tauri/src/{power_lifecycle,single_instance,windows_caption,windows_icon}.rs` | Power registration, process/window APIs, and window callbacks/resources | Callback context lifetime, process access limits, UI-thread HWND lifetime, and owned handles are checked at these platform edges. |

The scanner reports unsafe code under `cfg(test)` separately from production
unsafe. `rust/crates/sky_native_adapters/src/paths.rs` uses unsafe environment
mutation only in serialized resolver tests; no production path changes process
environment state.

Test seams are feature-gated or test-gated and never activated by environment variables alone in production. Production packages compose safe seams explicitly while runtime builds use disabled seams. There is no foreign-language extension crate, wheel build, or Python player binding in the workspace.

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
