# TAURI_DEV_WARNINGS_AUDIT: L18 Zero-Warning Elimination

**Repository:** `pumni/Sky-Auto-Player`  
**Branch:** `perf/dispatch-core-20261109`  
**Scope:** `sky_player` crate (Desktop Tauri dev profile)  
**Warning Elimination Policy:** Strictly 0 `#[allow(...)]`, `#![allow(...)]`, `-Awarnings`, dummy variables, or lint suppression. Every warning was resolved by fixing wiring, gating test-only symbols to `#[cfg(any(test, feature = "test-support"))]`, or safely removing unreferenced dead code.

---

## 1. Executive Summary

| Category | Initial Count | Remaining Count | Resolution Method |
| :--- | :---: | :---: | :--- |
| **Dead Code (`#[warn(dead_code)]`)** | 89 | **0** | Wired production pipeline (R1–R5), gated test helpers to test-only cfg, removed obsolete dead code |
| **Unused Imports (`#[warn(unused_imports)]`)** | 3 | **0** | Cleaned up obsolete imports in `worker.rs` and `mod.rs` |
| **Total Warnings** | **92** | **0** | **100% Eliminated with Root-Cause Corrections** |

---

## 2. Warning Triage & Resolution Table (Initial 92 Diagnostics)

| # | Diagnostic / Symbol | File & Line (Original) | Root Cause Analysis | Action / Resolution | Resulting State |
| :---: | :--- | :--- | :--- | :--- | :---: |
| 1–3 | `unused_imports`: `calibrated_spin_threshold_us`, `derive_spin_threshold_us`, `signed_ticks_to_us` | `src/engine/worker.rs:129–137` | Re-exports only used in test or `test-support` feature were unconditionally exported in production build | Gated re-exports under `#[cfg(any(test, feature = "test-support"))]` | Resolved (0 unused imports) |
| 4 | `struct DownBoundaryState` | `worker/dispatch/mod.rs:21` | Test-only diagnostic boundary struct retained in dispatch module root | Gated under `#[cfg(any(test, feature = "test-support"))]` with test coverage | Resolved |
| 5 | `struct PendingUpRecovery` | `worker/dispatch/mod.rs:37` | Retained recovery carrier struct without production caller | Gated under `#[cfg(any(test, feature = "test-support"))]` | Resolved |
| 6 | `struct PhysicalBoundaryStamp` | `worker/dispatch/mod.rs:48` | Boundary stamp helper struct retained for test harness | Gated under `#[cfg(any(test, feature = "test-support"))]` | Resolved |
| 7 | `fn drain_up_send_outcome` | `worker/dispatch/mod.rs:59` | Note-off release telemetry draining only constructed when test observer records Up observations | Moved to `observer_trace.rs` under `#[cfg(any(test, feature = "test-support"))]` | Resolved |
| 8 | `fn record_conflict` | `worker/dispatch/mod.rs:69` | Authored chord conflict telemetry only tracked in test harness verification | Gated under `#[cfg(any(test, feature = "test-support"))]` | Resolved |
| 9–10 | `fn wait_target_crossing_qpc`, `fn record_up_wait_attempt` | `worker/dispatch/mod.rs:94` | Legacy dynamic planner wait helpers unreferenced in shipping B executor loop | Cleanly removed dead functions without suppression | Resolved |
| 11–13 | `enum WaitBoundary::Replan` fields (`wait_result`, `target_qpc`, `planned_wait_ticks`) | `worker/wait.rs:16–19` | Replan variant carried debug fields that were never consumed by caller dispatch loop | Converted `WaitBoundary::Replan` to unit variant, eliminating unread fields | Resolved |
| 14 | `WaitBoundary::Due::dispatch_qpc` | `worker/wait.rs:15` | Pre-send wait boundary timestamp was unread in production dispatch loop | Wired to fallback `core.runtime.last_dispatch_deadline_wake_qpc` when `wait_result` is `None` (R3 evidence) | Resolved (wired in production) |
| 15–18 | `enum DownBoundaryAdmission`, `field target_crossing_qpc`, `field allow_pre_epoch_startup_dispatch` | `worker/dispatch/timing.rs:77–90` | Diagnostic timing state used by tests but not production loop | Gated under `#[cfg(any(test, feature = "test-support"))]` | Resolved |
| 19–21 | `fn physical_wait_target_for_plan`, `fn physical_boundary_stamp`, `fn physical_target_qpc_for_work` | `worker/dispatch_loop.rs:30–55` | Test-harness planning functions declared in dispatch loop module | Gated under `#[cfg(any(test, feature = "test-support"))]` | Resolved |
| 22–24 | `fn preflight_prepared_plan`, `fn dispatch_due_from_plan` | `worker/dispatch_loop.rs:135–180` | Legacy PreparedStream dispatch helpers retained in file | Gated under `#[cfg(any(test, feature = "test-support"))]` for characterization tests | Resolved |
| 25–48 | `DownMissObservation`, `UpObservation`, `BlockedUnfocusedObservation` and associated methods | `worker/dispatch/observation.rs` | Observer queue variants and test inspection helpers compiled in production | Gated under `#[cfg(any(test, feature = "test-support"))]` | Resolved |
| 49–75 | `DispatchPreparationProbe` methods (`record_admission`, `record_target_change`, etc.) | `worker/dispatch/observer.rs` / `observation.rs` | Instrumentation probe methods only evaluated by characterization test suite | Gated under `#[cfg(any(test, feature = "test-support"))]` while keeping production `record_input_build` active | Resolved |
| 76–84 | `StartupCheckpoint::StreamMaterialization`, `CoordinatorReset` | `engine/config.rs:220` | Boot checkpoints: `StreamMaterialization` was unwired; `CoordinatorReset` was dead | Wired `StreamMaterialization` to `attach_executor_with_cancellation` (R5); removed dead `CoordinatorReset` | Resolved (wired in production) |
| 85–92 | `WorkerRuntime::invalidate_down_authorization`, `take_deadline_wake_qpc`, `precision_handoff` | `worker/dispatch_loop.rs` & `observer_wake.rs` | Critical path functions for L18 findings R1, R2, R3 were orphaned in shipping B loop | Fully connected in production dispatch loop (`stage_candidate`, `dispatch_staged_candidate`) | Resolved (wired in production) |

---

## 3. Zero-Suppression Audit

In accordance with strict repository policy:
- Zero `#![allow(...)]` or `#[allow(...)]` attributes introduced. Specifically, `#[allow(clippy::too_many_arguments)]` on `dispatch_staged_candidate` and `drain_up_send_outcome` were replaced with dedicated domain context structs (`StagedDispatchContext`, `UpObservationDrainContext`).
- Zero dummy variables (`_foo`), dummy use statements, or blank implementations.
- No `-A warnings` or build flag adjustments.
- All 92 dead code / unused import diagnostics are resolved at the root cause.
