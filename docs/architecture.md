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
integration. Playback integration keeps the existing `#[path]` entrypoint and
the `playback_integration.rs` root as a thin module declaration file. Its test
owners under `native_runtime/tests/playback_integration/` are:

- `supervisor_lease.rs`: monitor progress, stalls, supervisor startup, and
  lease expiry.
- `terminal_release.rs`: worker retirement and join, cleanup, release proof,
  quarantine, and physical reservation.
- `presentation_events.rs`: lifecycle event ordering, queue behavior, and
  critical presentation.
- `diagnostics_trace.rs`: diagnostics status and gates, sender trace, and
  health projection.
- `start_prepare.rs`: startup admission, validation, prepare, IDs, fingerprints,
  and cancellation.
- `storage_bootstrap.rs`: install and app-data roots, cache, settings, and
  source mutation.
- `commands_status.rs`: status DTOs and stop, skip, pause, and command behavior.

The architecture audit keeps the root at or below 500 lines, each owner at or
below 1,600 lines, and requires the exact direct `.rs` sibling inventory.
The 69-test baseline inventory, feature gates, nested helper, and per-test
body hashes are recorded in
`docs/evidence/issue-540-native-playback-test-parity.json`.

The `sky_player` engine suite remains entered through `engine.rs`'s
`#[cfg(test)] mod tests;`. Its `engine/tests.rs` root contains the shared test
imports and module declarations and stays at or below 750 lines. Shared fixtures
and builders belong to `engine/tests/support.rs`. The semantic test owners are:

- `startup_admission.rs`: startup ordering, readiness and armed publication,
  pre-roll, and startup failure handling.
- `lease_supervision.rs`: supervisor lease progress, expiry, and terminal ownership.
- `preparation_dispatch.rs`: schedule preparation, preparation checkpoints,
  stale packets, and dispatch admission.
- `release_cleanup.rs`: terminal outcomes, scoped release, and failure cleanup.
- `native_dispatch.rs`: native session send paths, packet faults, and dispatch
  contracts.
- `telemetry_metrics.rs`: telemetry records, traces, metrics, and mixed-packet
  event cardinality.
- `target_focus_admission.rs`: target identity and focus validation, plus final
  down admission.
- `pause_suspend.rs`: pause, resume, suspend, and transition races.
- `timing_wait.rs`: timing policies, deadlines, wait paths, and join behavior.

The architecture audit enforces the exact direct `.rs` owner inventory and a
2,200-line ceiling for each owner.

### Desktop frontend state ownership

The desktop keeps one canonical Zustand store. `DesktopStore` in
`desktop/src/state/types.ts` is its state and action contract, and
`createDesktopStore(bridge)` in `desktop/src/state/store.ts` is the composition
root. The typed bridge remains the frontend boundary; feature controllers do
not create separate stores or synchronize state through effects.

- `settingsSlice.ts` owns settings patching and serialized settings updates.
  `updateSlice.ts` owns update check and install handoff request state, dialog
  state, and frontend transport errors. Rust `UpdateService` remains the
  authority for update policy and native update state.
- Native settings own `appearance_mode` and its legacy migration alongside the
  dark palette. `App.tsx` resolves System from one platform source at a time
  (Tauri window theme/events when packaged, browser media query otherwise) and
  applies it before the normal startup reveal; the bounded 2.5-second fail-open
  can reveal the OS-media startup screen before native settings arrive when
  bootstrap is delayed. The effective mode is not stored in browser storage.
  The final A–C integration audit, qualification matrix, and complete changed-path
  inventory are recorded in
  [issue-565 Package D evidence](evidence/issue-565-package-d/README.md).
- `librarySlice.ts` owns Library query intent, bounded page caching,
  virtualization hydration, and paging. `librarySelectionActions.ts` owns
  selected-song/detail requests and browser navigation.
  `libraryMutationActions.ts` owns playlist and Like mutations, their
  latest-intent behavior, and navigation request fencing. Its mutation ledger
  is shared with query/cache reconciliation so optimistic Likes and membership
  revisions remain consistent with cached pages.
- `libraryCatalog.ts` owns catalog-generation reset and bounded catalog
  reconciliation. `libraryPlaybackCoordinator.ts` is the composition seam for
  cross-feature state changes: catalog invalidation, membership invalidation,
  browser-navigation projection, and playing-song Like projection. It stores
  no second Library or Playback copy.
- `playbackSlice.ts` composes Playback state transitions and typed playback
  event projection. `playbackPreparation.ts` owns playback-context row
  resolution and native prepare requests. `playbackSessionLedger.ts` owns
  pending start identities, start epochs, retirement history, and retirement
  waiters. `playbackReconciliation.ts` owns transport-operation epochs,
  timeouts, and native status recovery. `playbackTransport.ts` owns transport
  actions, while `playbackSourceTraversal.ts` owns frozen-source traversal and
  the natural Auto Play handoff.

The controllers mutate the same store through the composition root. Library
paging limits, selection versus playing identity, Playback session retirement,
and native updater authority remain feature invariants.

### Desktop UI composition and verification ownership

The frontend view tree composes the state owners; it does not introduce a
second domain store or take authority from `DesktopBridge`:

- `desktop/src/components/workbench/Workbench.tsx` composes the Navigator, Track Browser,
  Inspector, and their resize separators only. `desktop/src/app/App.tsx` owns the
  `.app-shell` view composition and renders `<Workbench />` and `<PlayerBar />` as sibling
  children. `desktop/src/components/player/PlayerBar.tsx` owns the fixed 92 px Player Dock,
  outside the Workbench subtree. `desktop/src/components/workbench/workbenchGeometry.ts`
  owns the pure geometry calculation and the shared 420 px Track Browser minimum.
  `desktop/src/components/workbench/useWorkbenchLayout.ts` applies measured viewport geometry and input state;
  `desktop/src/components/workbench/workbenchDragPolicy.ts` owns transient drag/topology rebasing.
  `desktop/src/components/workbench/useWorkbenchPreferences.ts` coordinates UI preference updates,
  while `desktop/src/state/workbenchPreferences.ts` owns v5 validation, normalization, and
  persistence. Only explicit completed preference changes are persisted;
  resize-derived overlay/collapse state remains transient.
- `desktop/src/components/settings/SettingsPanel.tsx` owns the modal, category navigation,
  and focus lifecycle. `desktop/src/components/settings/SettingsCategoryContent.tsx` owns the
  category content and palette choices. Settings writes continue through the existing store and
  typed bridge; the split does not change the settings schema or native owner.
- `desktop/src/components/utility/DiagnosticsView.tsx` composes the Diagnostics view.
  `desktop/src/components/utility/diagnosticsPresentation.ts` owns pure presentation mapping, and the playback
  summary, readout, event, timing, and performance components own their bounded
  render surfaces. Reported session, sender, and timing facts remain sourced
  from native state; unavailable and dry-run measurements are not fabricated.

Regression ownership follows those boundaries. `desktop/src/state/store.test.ts` covers
request fencing, generation-aware bounded Library caches, selection versus
playing identity, page hydration, and Playback preparation/session lifecycles.
`desktop/tests/e2e/library.spec.ts` and `desktop/tests/e2e/playback.spec.ts` cover visible paging,
full-source traversal, and cross-page selection/playback behavior.
`desktop/src/components/workbench` unit tests plus
`desktop/tests/e2e/workbench-layout.spec.ts`, `workbench-resize.spec.ts`, and
`workbench-state-regression.spec.ts` cover geometry, drag, keyboard/ARIA state, and
v5 persistence. Settings, Diagnostics, palette, and Graphite appearance suites
cover modal/accessibility behavior and all palette consumers. Browser bridge
tests establish frontend behavior only; they do not replace packaged Tauri,
Windows focus, or physical-input qualification.

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

| Module                                                                                                                                                                | Unsafe operation                                                        | Safety invariant                                                                                                                  |
| --------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------- |
| `rust/crates/sky_dispatch_win32/src/{calibration,clock,cpu,event,focus,focus/window_identity,input/packet,input/physical,input/raw,mmcss,power,timer,wait/hybrid}.rs` | Win32 handles, callbacks, timing, packet construction, and `SendInput`  | Platform calls stay in the Windows adapter; owned handles are closed by their owner and physical input uses `SendInput` only.     |
| `rust/crates/sky_native_adapters/src/lib.rs`                                                                                                                          | `MoveFileExW` atomic settings replacement                               | NUL-terminated source and destination buffers stay live through the synchronous replace call.                                     |
| `desktop/src-tauri/src/{power_lifecycle,single_instance,windows_caption,windows_icon}.rs`                                                                             | Power registration, process/window APIs, and window callbacks/resources | Callback context lifetime, process access limits, UI-thread HWND lifetime, and owned handles are checked at these platform edges. |

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

The xtask ownership root in `rust/xtask/src/checks.rs` keeps the check-group
dispatch order, check-session verification memoization, supply-chain skip
policy, and desktop/native orchestration. Focused owners contain the validators:

- `audits/security.rs` owns the forbidden API, DLL, and Windows module scanner
  and its baseline behavior.
- `audits/architecture.rs` owns dependency direction, architecture allowlist,
  module sizing, unsafe inventory, and module budget checks.
- `audits/source.rs` owns shared Rust comment stripping and the shared source
  walker used by audit and release-metadata validation.
- `checks/tauri_metadata.rs` owns Tauri feature/build freshness, release
  metadata, and legacy topology contracts.
- `checks/release_runner.rs` owns protected runner, release workflow, and
  metadata-token scope contracts.
- `checks/release_pipeline.rs` owns release-state ordering and production
  qualification contracts; `checks/workflow.rs` owns shared workflow parsing
  and state-invocation validation.
- `checks/ci_contracts.rs` owns active CI wiring and packaged-CI contracts;
  `checks/trust.rs` owns production calibration and v4 trust-material guards.
- `checks/bindings.rs` owns TypeScript and IPC binding generation and checks.

The architecture audit budgets each owner module and requires the root
`checks.rs` to remain at or below 1,600 lines. It also enforces the `sky_player`
engine test root and owner budgets, plus their exact sibling inventory, and the
native playback integration root, owner budgets, and exact sibling inventory.

Use the Rust repository tool for current verification groups:

```powershell
cargo xtask check static
cargo xtask check rust
cargo xtask check desktop
cargo xtask check all
```

The `xtask` retirement, architecture, security, branding, and zero-Python checks are canonical.
