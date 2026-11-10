# Playback Verification and Qualification Protocol

This document defines the normative qualification protocol for the Sky Auto Player
realtime playback engine. All changes affecting scheduling, admission, timing guards,
or Win32 input boundaries must pass this protocol prior to release qualification.

## 1. Identity-Bound Native Sink

Native input testing requires an identity-verified `ReceiveOnly` sink window
managed via `scripts/native_acceptance_sink.ps1`:

- **Process Isolation:** The sink runs in an independent process (`native_sink.exe`).
- **Identity Binding:** The qualification harness records the sink window's HWND,
  process ID, process creation timestamp, and binary path.
- **Interactive Desktop Requirement:** Real `SendInput` testing requires an active,
  interactive desktop (`WinSta0\Default`) where `GetForegroundWindow() == sink_hwnd`.
- **Fail-Closed Sandbox Guard:** In headless or non-interactive environments where
  `GetForegroundWindow()` returns `NULL`, the harness safely denies input dispatch
  and reports `QUALIFICATION INCONCLUSIVE`. Headless runs are **never** counted as
  a native safety PASS.

## 2. The 22 Native Safety Scenarios

The suite validates 22 edge cases and fault injections under both Quiet and
Contention (two `BelowNormal` priority CPU worker threads) loads:

| Scenario | Class | Invariant Verified |
| --- | --- | --- |
| `canonical-single` | Baseline | Single-note timing, $H$, $G$, and clean release |
| `canonical-max-chord` | Capacity | Full 15-key chord dispatch in a single `SendInput` packet |
| `near-minimum-retrigger` | Timing | Retriggering at exactly $H + G$ boundary without overlap |
| `release-gap-stress` | Timing | Dense rapid sequences maintaining frame release gap $G$ |
| `mixed-up-down` | Ordering | Coalesced Up and Down batch dispatches Up-before-Down |
| `ambiguous-packet` | Robustness | Malformed chord definitions rejected without dispatch |
| `preflight-user-held` | Preflight | Physical keys held by user prior to session abort start |
| `modifier-held-final-boundary` | Admission | Active Ctrl/Alt/Shift at final boundary blocks Down send |
| `modifier-held-after-owned` | Admission | Modifier pressed during active playback triggers safe release |
| `focus-loss` | Focus | Focus loss halts Down send and triggers Up cleanup |
| `target-hwnd-change` | Authority | Background window replacement invalidates Down authority |
| `owner-mismatch` | Authority | HWND reassigned to different PID halts Down transmission |
| `owner-query-failure` | Authority | Failure querying foreground owner fails closed before send |
| `owner-process-termination` | OS Termination | OS-level `TerminateProcess` halts preroll; zero Down sender attempts |
| `pause-resume` | Lifecycle | Musical time freezes; resume enforces $Q + T$ barrier |
| `pause-resume-release-floor` | Lifecycle | Resume preserves unexpired $H$ floor for in-flight keys |
| `suspend-resume` | System Power | Windows APM suspend notification triggers terminal abort |
| `stop-cleanup` | Teardown | User stop releases all active key slots cleanly |
| `skip-cleanup` | Teardown | Track skip clears pipeline and releases keys before next track |
| `supervisor-lease-expiry` | Watchdog | Realtime stall triggers supervisor lease invalidation |
| `lease-age-watchdog-delayed` | Watchdog | Stale lease age blocks Down send at final admission boundary |
| `noncanonical-profile` | Config | Arbitrary FPS and timing margins adhere to $H$ and $G$ rules |

### OS Termination and Strict Down Attempt Invariant
For `owner-process-termination`, qualification requires external proof of real
Win32 `TerminateProcess` execution and strictly enforces `down_sender_attempt_count == Some(0)`.
Synthetic handle invalidation is explicitly rejected.

## 3. Zero-Allocation (Noalloc) Qualification

Realtime dispatch loops must never perform heap allocations during healthy playback:
- The `noalloc_qualification` suite in `rust/crates/sky_dispatch_core/tests/` verifies
  that `DispatchExecutor` operations (chord iteration, release due calculation,
  disjoint batch collection) execute with zero heap allocations.
- Dynamic structures (e.g. song streams) are compiled ahead of playback; runtime
  state operates on fixed 15-slot stack/inline buffers.

## 4. Execution Workflow

Qualification is run using immutable harness executables:

```powershell
# 1. Build immutable qualification harness
powershell -NoProfile -File scripts/build_dispatch_core_harness.ps1 -Profile dist -GuardMode frozen

# 2. Run automated native qualification suite
powershell -NoProfile -File scripts/qualify_dispatch_core.ps1 `
  -Stage Smoke `
  -HarnessPath path\to\rt-native-acceptance.exe `
  -HarnessManifestPath path\to\rt-native-acceptance.manifest.json
```

Reports, CSV session ledgers, and telemetry traces are generated in
`evidence/dispatch-core-646/`. All 22 scenarios must pass before release.
