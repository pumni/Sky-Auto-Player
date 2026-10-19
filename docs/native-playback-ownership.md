# Native Playback Ownership

This document records the ownership split for the Tauri native playback
control plane. The physical dispatch contract remains in
[`rt-dispatch-architecture.md`](rt-dispatch-architecture.md) and
[ADR-0018](adr/ADR-0018-resumable-release-barrier-and-final-lease-age.md).

## Before and after

Before this extraction, `desktop/src-tauri/src/native_runtime.rs` contained the
Tauri composition root, prepared-plan cache, active session, supervisor,
per-session presentation worker, diagnostics projection, terminal retirement,
sender trace, and the shared app event hub in one module.

Playback ownership now follows this dependency graph:

```text
NativeDesktopRuntime                         native_runtime.rs
  ├─ settings, catalog, calibration, updater  remain app-scoped
  ├─ Tauri playback command routing           calls NativePlaybackService
  └─ NativeEventHub                            app-wide 128-event queue + one async publisher
       ▲                                  ▲
       │                                  │ bounded shared UiEvent delivery
       │                                  │
native_runtime/playback/                  │
  ├─ mod.rs                                │ shared session state and module facade
  ├─ session.rs ───────────────────────────┘ prepared plans, controls, monitor, trace
  ├─ presentation.rs                          per-session bounded projection/consumer
  ├─ diagnostics.rs                           gate, sampling and DTO projection
  └─ retirement.rs                            join, release proof, quarantine and handoff
       │
       └─ NativeDispatchSession                sky_player worker and physical sender
```

There is one physical dispatch worker per physical session, one real safety
supervisor, one per-session presentation consumer, and one app-wide asynchronous
UI event publisher. The extraction adds no sender, service container, queue, or
safety authority.

## Owners, locks, and lifetimes

| Owner | State and synchronization | Responsibility |
|---|---|---|
| `NativePlaybackService` (`playback/mod.rs`, methods in `session.rs`) | `prepared: Mutex<VecDeque<_>>`, `active: Arc<Mutex<Option<Arc<NativeActivePlayback>>>>`, `last_terminal`, `sender_trace_state`, diagnostics gate | Bounded prepared-plan cache; one active UI session; last terminal status; session trace; diagnostics enable/disable ordering. |
| `ActivityCoordinator` and active session | `activity_lease`, session `state`, pending control, power request and suspend registration mutexes; atomic stop/done/quiescent/release flags | Physical activity reservation; pause/resume/stop/skip intent; OS power and suspend resources; session terminal visibility. |
| `NativeDispatchSession` | Worker-owned scheduler, sender, timing guards, retained target/owner proof and tracked release state | Sole physical `SendInput` path. Existing one-attempt-per-packet, exact target, supervisor lease, final Down and H/G contracts stay here. |
| `NativePlaybackService::monitor` (`session.rs`) | Monitor thread observes active session and calls the existing player APIs | Fresh focus, owner and control observation; safety progress and three-second supervisor lease renewal. It does not become a heartbeat-only thread. |
| Per-session presentation (`presentation.rs`) | One-slot `latest_snapshot` mutex; atomics for order, coalesced/dropped snapshots and failure; 64-slot synchronous critical lifecycle queue; owned worker handle | Projects snapshots and serializes critical presentation work without entering the sender suffix. Critical queue/transport failures still request fail-closed cleanup. |
| Diagnostics (`diagnostics.rs`) | `DiagnosticsPublicationGate` mutex holds the bounded publication linearization point; sample/DTO values are local to the consumer | Cadence, monotonic diagnostics sequence, disable/reenable reset, status and statistics projection. Sampling stays detached from the safety monitor. |
| Terminal retirement (`retirement.rs`) | Active slot and release/reservation state are updated only after worker outcome and quiescence proof | Join, tracked release verification, quarantine on uncertain cleanup, terminal event barrier, then successor admission. |
| `NativeEventHub` (`native_runtime.rs`) | App-wide queue state mutex, 128 shared event slots, buffered snapshots and one publisher worker | Shared catalog/calibration/playback event transport. Platform `Channel::send` remains on its asynchronous publisher; the hub does not own per-session snapshots or supervisor authority. |

`session_id` identifies the UI playback session. The presentation queue's
monotonic `order` preserves lifecycle/snapshot ordering within that session;
the active snapshot `seq` remains session scoped. Diagnostics use their own
monotonic `seq` and gate epoch. The app event queue retains its existing
snapshot coalescing and lifecycle ordering rules.

Settings and catalog generation changes still invalidate prepared plans at the
composition call sites. Catalog enumeration, updater, calibration, Tauri shell
construction, and the global event hub remain outside the playback modules.
The composition root still routes prepare, start, session command, status,
diagnostics toggle, trace export, settings/catalog invalidation, and shutdown
to the playback service.

## Deprecated counter audit and compatibility

`WorkerMetricsLocal` had storage for four counters that no production path
wrote. The source audit found no assignments or increments, and the retired
decision branches cannot produce them. That dead local storage and its
test-only accessors are removed. Active final-gate, floor-delay, H/G forensics,
release, lease, and anomaly-ring evidence is unchanged.

| Counter | Producer/storage before | Current internal state | Preserved surface and projection |
|---|---|---|---|
| `missed_physical_window_boundaries` | No writer; deprecated field in `WorkerMetricsLocal` copied into snapshots | Local field removed | Public `EngineSnapshot`, `EngineProgressSnapshot`, Tauri `DiagnosticsSnapshotDto`, generated TypeScript type, and UI consumer keep the same `u64`/JSON number field. Rust and Tauri projection write `0`. |
| `final_sender_window_expirations` | No writer; deprecated field in `WorkerMetricsLocal` copied into snapshots | Local field removed | Same public snapshot and Tauri/UI fields remain with numeric zero. |
| `release_floor_infeasible_boundaries` | No writer; release floors delay eligible packets and do not reject a Down | Local field removed | Same public snapshot and Tauri/UI fields remain with numeric zero. |
| `prepared_normal_sender_expirations` | No writer; worker-local only, not a public snapshot or Tauri field | Local field and test accessor removed | No wire field existed. Prepared backlog and Up-prefix recovery counters remain. |

Rust snapshot field names, order, types, and presence are unchanged. The Tauri
DTO serializer continues to emit the three public compatibility keys as
numbers, including unavailable/inactive samples. Nullable diagnostics retain
their existing null behavior; no property was changed to absent. The generated
`DiagnosticsSnapshotDto` TypeScript declaration and UI rendering code are
unchanged. Mock UI fixtures with nonzero legacy values continue to exercise
historical/display behavior and are not production counter writers.

## Verification and timing scope

The move preserves the existing deterministic tests for Ready/Arming/Armed,
dry-run and physical ownership, focus/owner/pause/stop/skip/panic/suspend,
supervisor stall, presentation and channel stalls, critical overflow,
diagnostics linearization, trace export, event delivery failure, join timeout,
release quarantine, and successor exclusion. Golden Tauri serialization tests
continue to check inactive/deprecated zeros and null-versus-present fields.
The held-frame ordering regression uses a test-only barrier after work selection,
so its coalescing assertion does not depend on polling a contended mutex. The
real watchdog expiry fixture uses the production three-second lease value to
avoid expiring during startup on a loaded test runner.

The physical dispatcher, `SendInput` call boundary, final Down policy, H/G
floors, target generation, lease expiry, packet grouping, and timing
configuration are unchanged. The diagnostics compatibility projection replaces
dead internal counter reads with literal zeroes; it adds no work to the QPC to
`SendInput` suffix. The extraction does not change physical startup or
retirement wiring, so this package does not require a new foreground ReceiveOnly
smoke or historical A/B/performance campaign. Existing native evidence remains
applicable to the unchanged physical composition; game receiver behavior is
not requalified by this source-only change.
