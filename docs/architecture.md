# System Architecture

The Tauri/React desktop is the only supported user-facing product. Its
commands are handled by the native Rust composition root; the repository has
no separate scripting runtime in the packaged application.

## Repository layout

```text
apps/desktop/       React + Vite frontend and Tauri shell
apps/website/       independent static Astro site for GitHub Pages
rust/               Cargo workspace: domain, native adapters, desktop shell, xtask
songs/              built-in sheet files packaged by the desktop app
builtin-songs/      immutable built-in catalog manifest and metadata
scripts/            repository and release qualification helpers
```

The Rust workspace remains under `rust/` because its Cargo config, lockfile,
target directory, and workspace membership define one shared native build
boundary. The Tauri crate joins that workspace through its path at
`apps/desktop/src-tauri`; package names such as `sky_desktop_shell` and
`sky_app_core` remain stable responsibility names used by Cargo commands and
CI. Desktop and website keep their own `package.json` and `bun.lock` so each
app has a frozen, independently validated dependency graph. There is no root
Bun workspace or duplicate root JavaScript manifest.

The website builds static Pages output without invoking Cargo or Tauri. The
desktop Vite bundle is consumed by the Tauri shell, which packages the Rust
workspace output and the catalog resources. Local command roots and CI lane
ownership are recorded in [desktop tooling](desktop-tooling.md) and
[CI validation topology](ci-validation-topology.md).

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
- `sky_desktop_shell` (`apps/desktop/src-tauri`) owns Tauri command decoding, delivery DTO translation, and the Rust-owned `UpdateService` policy boundary. It contains no song scheduling, risk analysis, persisted-settings migration, or realtime dispatch algorithms.

## Native desktop ownership

All 32 stable request/response commands have one native handler. The separate
`subscribe_ui_events` Tauri command establishes the UI event subscription; it
is not a request/response method in the IPC method table. The production
router has no auxiliary process or automatic fallback. `core.ready` remains a
compatibility wire event name and is produced by the native runtime where the
frontend contract requires it.

`NativeDesktopRuntime` in `apps/desktop/src-tauri/src/native_runtime.rs` remains the
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
the official Tauri updater. Native runtime tests cover catalog and library
behavior, calibration, event delivery, and playback integration. The
`sky_player` engine tests cover startup and readiness, lease supervision,
preparation and dispatch, release and cleanup, native send paths, telemetry,
target and focus admission, pause and suspend, and timing.

Rust module declarations and Cargo test discovery determine which modules are
compiled and exercised; the directory layout is an organizational choice. The
architecture audit reports module size and context-field counts as advisory
warnings. Timing-sensitive worker and dispatch limits, dependency direction,
dispatch topology, and safety boundaries remain enforced.

### Desktop frontend state and UI ownership

The desktop keeps one canonical Zustand store behind the typed bridge. Feature
state and controllers compose through that store; they do not maintain duplicate
Library or Playback state. Settings writes and update handoff requests cross the
existing bridge, while Rust `UpdateService` remains the authority for update
policy and native update state.

Native settings own `appearance_mode` and its legacy migration. System appearance
uses one platform source at a time (Tauri window events when packaged, browser
media query otherwise), is applied before normal startup reveal, and is not
stored in browser storage. A bounded 2.5-second fail-open may reveal the
OS-media startup screen while native bootstrap is delayed.

Library state preserves bounded paging, selection versus playing identity,
latest-intent playlist and Like mutations, and generation-aware cache
reconciliation. Playback state preserves session identity and retirement,
operation fencing, native status recovery, frozen-source traversal, and the
natural Auto Play handoff. Workbench layout and drag state remain transient;
only completed preference changes are persisted. Settings retain their modal
focus lifecycle, and diagnostics present native measurements without fabricating
unavailable or dry-run data.

Frontend unit and browser tests cover store behavior and visible workflows.
They establish browser-side behavior only; they do not replace packaged Tauri,
Windows focus, or physical-input qualification.

## Playback boundary

```text
song file -> pure Rust parser/planner -> native playback adapter
          -> sky_player -> dispatch_core/dispatch_win32 -> SendInput
```

Playback ownership and realtime behavior are specified in
[playback-engine.md](playback-engine.md); native and gameplay qualification is
specified in [playback-verification.md](playback-verification.md). This overview
records only the repository boundary: `sky_player` owns playback lifecycle and
orchestration, `sky_dispatch_core` owns musical scheduling policy, and
`sky_dispatch_win32` owns physical Windows effects. Dry-run playback does not
reach the physical input backend.

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
exact installer/signature bytes re-downloaded from a release draft.
The retired v3 transaction/recovery implementation is preserved in Git history
and the `v3-maintenance` line; it is not part of the current workspace or
product graph.

## Unsafe boundary and test seams

Production unsafe Rust is limited to the reviewed platform boundaries below.
The newly audited desktop and native-adapter blocks carry local `// SAFETY:`
justifications; the existing Win32 adapter modules remain separately listed in
the architecture allowlist. Application orchestration, domain policies
(`sky_app_core`), Tauri command delivery, catalog indexing, calibration
validation, and updater policy remain safe Rust.

| Module                                                                                                                                                                | Unsafe operation                                                        | Safety invariant                                                                                                                  |
| --------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------- |
| `rust/crates/sky_dispatch_win32/src/{calibration,clock,cpu,event,focus,focus/window_identity,input/packet,input/physical,input/raw,mmcss,power,timer,wait/hybrid}.rs` | Win32 handles, callbacks, timing, packet construction, and `SendInput`  | Platform calls stay in the Windows adapter; owned handles are closed by their owner and physical input uses `SendInput` only.     |
| `rust/crates/sky_native_adapters/src/lib.rs`                                                                                                                          | `MoveFileExW` atomic settings replacement                               | NUL-terminated source and destination buffers stay live through the synchronous replace call.                                     |
| `apps/desktop/src-tauri/src/{power_lifecycle,single_instance,windows_caption,windows_icon}.rs`                                                                             | Power registration, process/window APIs, and window callbacks/resources | Callback context lifetime, process access limits, UI-thread HWND lifetime, and owned handles are checked at these platform edges. |

The scanner reports unsafe code under `cfg(test)` separately from production
unsafe. `rust/crates/sky_native_adapters/src/paths.rs` uses unsafe environment
mutation only in serialized resolver tests; no production path changes process
environment state.

Test seams are feature-gated or test-gated and never activated by environment variables alone in production. Production packages compose safe seams explicitly while runtime builds use disabled seams. The workspace builds no extension modules or alternate player bindings.

## Evidence and repository tooling

Production behavior is established by checked-out source, direct Rust and
TypeScript tests, and committed fixtures. Historical records and frozen data
preserve evidence for their recorded revisions; they do not set current
architecture by themselves. Rust and Bun are the canonical product and
development toolchains.

`cargo xtask` dispatches verification groups and hosts architecture, security,
desktop-boundary, documentation, supply-chain, packaging, and release/trust
checks. These checks enforce behavioral and security contracts; module-size
and context-field diagnostics are advisory.

Use the Rust repository tool for current verification groups:

```powershell
cargo xtask check static
cargo xtask check rust
cargo xtask check desktop
cargo xtask check all
```

The `xtask` architecture, security, branding, and runtime-boundary checks are canonical.
