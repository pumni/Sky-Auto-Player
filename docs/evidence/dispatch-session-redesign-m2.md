# Dispatch Session Redesign M2 Evidence

M2 implements one prepared schedule boundary on top of accepted M1
(`362d19fb871d2983d58e1d01457c612b2e33105a`). M2 is submitted for Leader
review; this record does not authorize M3.

## Caller and deletion map

| Symbol or path | Caller before | Caller after or deletion | Verification |
| --- | --- | --- | --- |
| `NativeScheduleInput::{Raw, Prepared}`; `NativeSessionOptions::schedule`; `engine/session.rs`; `engine/worker.rs`; `engine/worker/boot.rs` | Session options could carry either a `RuntimeSchedule` or a prepared plan; boot could materialize the raw schedule during Arm. | Removed the enum and raw branch. Session options, admitted options, and the worker now carry only `PreparedDispatchSchedule`. Admission and worker boot each check QPC frequency and H/G compatibility; worker boot retains the existing capacity-budget gate and consumes the plan without rebuilding it. | `prepared_schedule_passes_session_admission_and_worker_arm_without_rematerializing`; prepared-plan compatibility tests; default/all-feature Rust suites. |
| Desktop Start: `apps/desktop/src-tauri/src/native_runtime/playback/session.rs` | Desktop Prepare already built and retained a prepared plan; Start wrapped it in `NativeScheduleInput::Prepared`. | Start moves the same prepared plan into native session options. The plan is not recompiled or materialized at Start. | Desktop-native check; prepared-plan session/worker regression. |
| Desktop native test callers: `native_runtime/tests.rs`, `native_runtime/tests/playback_integration/supervisor_lease.rs`, `power_lifecycle.rs` | Test-only sessions passed `RuntimeSchedule` through `.into()`. | Each test prepares its schedule with the test's QPC frequency and H/G values before constructing `NativeSessionOptions`. | Default/all-feature Rust checks; `cargo xtask check desktop-native`. |
| Engine test support: `engine/tests/support.rs`, `engine/config/effective_policy.rs`, `engine/tests/startup_admission.rs`, `engine/tests/telemetry_metrics.rs` | Shared helper created raw options; one regression tested cancellable worker-time materialization. | Shared helpers prepare before options; the wider-margin telemetry fixture prepares with its final H/G values. The old worker-materialization test/counters/checkpoint were removed and replaced with a Prepare-cancellation regression that completes before session construction. | `prepared_schedule_cancellation_completes_before_session_construction`; `prepared_schedule_passes_session_admission_and_worker_arm_without_rematerializing`; all-feature suite. |
| Direct dispatch test harness: `engine/test_support/dispatch_harness.rs::configure_focus_owner_for_test` | Default `sky_player` unit tests compiled the harness but bound only an HWND/PID pair, while M1's focused final gate requires retained process authority. | Under unit tests and `test-support`, the harness binds the same synthetic retained authority as feature builds; production admission and safety gates are unchanged. | Default and all-feature focus/owner regression suites. |
| Acceptance: `bin/acceptance/scenarios.rs`, `modifier_guard.rs`, `bin/rt_native_acceptance.rs` | Scenario plans retained `RuntimeSchedule` until `NativeSessionOptions` construction, where `.into()` selected the raw worker route. | Acceptance options now prepare with the current QPC frequency, scenario H/G, and compile-retained-capacity accounting before session construction. Preparation errors produce an Inconclusive report; no worker is created. | Acceptance option tests and all-feature checks/tests. No ReceiveOnly or live-input acceptance was run. |
| Timing Advisory: `timing_model/dispatch_schedule.rs`; Desktop `playback/timing_advisory.rs` | Live Desktop Prepare analyzed the compiled `RuntimeSchedule` through `DispatchExecutor::new_with_cancellation`; dedicated timing-model tests use the same API. | Kept as the offline advisory engine. It does not feed a worker and retains its independent advisory oracle. No second production playback route remains. | Timing-model tests in the default/all-feature suites; Desktop-native check. |
| `examples/dispatch_lifecycle_evidence.rs` and `scripts/run_dispatch_lifecycle_evidence.ps1` | Example exercised `ProductionDispatchTestHarness` directly and did not construct `NativeSessionOptions` or accept raw schedules. | No migration was required; it remains a mock/test-support lifecycle example with the prepared dispatch harness. | All-feature example/build checks and `cargo xtask check desktop-native`. |
| Win32 sender `*_with_start` surface: `input.rs`, `input/packet.rs`, `input/tracked/packet_send.rs` | Public re-exports and public tracked methods exposed timestamp-injection helpers; production legacy `send_physical_packet` and unit tests used the same implementation. | Removed public re-exports. The prepared timestamp-injection methods and borrowed-view helper are test-only; the remaining timestamp-carrying helper is crate-private for the existing `send_physical_packet` path, preserving its QPC sample and one-attempt behavior. | Win32 packet/tracked input tests; raw-zero, partial, and QPC evidence tests; default/all-feature suites. |
| Win32 raw and target-bound helpers: `input/raw.rs`, `input/packet.rs`, `calibration.rs`, `input/tracked/packet_send.rs` | Raw SendInput remains used by tracked cleanup and calibration; the target-bound prepared packet helper is used by calibration's final target gate. | Kept these live paths, including safety Up/cleanup. No caller was migrated to an untracked musical route. | Win32 unit tests and the Rust suites. This is unit/mock evidence, not native playback qualification. |
| Owner poll result: `OwnerIdentityPollResult`, `poll_window_identity_authority_result`; Desktop consumer tests | The bool polling API was already absent at the accepted M1 base. `StaleSuperseded` remained as an enum variant and a consumer-only no-op test, but no poll path emitted it. | Removed `StaleSuperseded` and its synthetic consumer test. The live result has only `CurrentValid` and `CurrentInvalid`; Desktop calls the result API. | `rg` caller census; current-valid/current-invalid poll regressions and all-feature suites. |
| Wait guard configuration: `WaitOptions`, `EffectiveRuntimePolicy`, `engine.rs` exports | Runtime structs carried the single-value `WaitGuardMode::Frozen`; it had no runtime decision branch. | Removed the runtime field, effective-policy copy, and `sky_player::engine` re-export. Win32's `WaitGuardMode` parser/label remains for CLI compatibility. `--guard-mode` and `SHIPPING_WAIT` are parsed as compatibility input only; runtime policy remains Frozen. | New CLI compatibility tests; Win32 frozen/adaptive parser tests; script census below. |
| Qualification CLI and PowerShell: `scripts/run_native_acceptance_case.ps1`, acceptance `arguments.rs`; qualification manifest readers | The runner copies manifest `guard_mode` into `--guard-mode`; manifest/qualification readers use the value to validate supported artifacts and profiles. | Kept script and manifest formats unchanged. CLI accepts `frozen` and rejects retired `adaptive`, but does not configure runtime. | `frozen_guard_cli_input_remains_accepted_for_harness_compatibility`; `retired_adaptive_guard_cli_input_is_rejected`; existing PowerShell profile/manifest validation remains unchanged. |
| Historic qualification tooling: `scripts/build_dispatch_core_harness.ps1`, `scripts/qualify_dispatch_core.ps1` | Harness manifests and the comparator use `guard_mode` to preserve pinned Frozen/Adaptive profile identity, including old artifact reads. | Kept the manifest field, profile validation, comparator, and artifact reader unchanged; no runtime waiter configuration is restored. | Existing script/artifact contract census; no PowerShell file changed. |
| Feature gates: `sky_player/Cargo.toml`, acceptance bin, no-allocation integration, Desktop | `real-input-acceptance` implies `test-support`; acceptance CLI and no-allocation integration have required features; Desktop checks use `desktop-runtime` and `tauri-test`. | Prepared schedule callers compile in the same feature paths. Test-only owner authority and QPC seams remain behind `cfg(test)`/`test-support`; production options no longer export `WaitGuardMode` or raw schedules. | Default and all-feature Rust checks/tests; `cargo xtask check desktop-native`. |
| `TestWaitPolicy::LegacyTestWideSpin` and `test_spin_threshold_us` | Mock engine and Desktop tests explicitly used the wide-spin policy; acceptance production uses `ProductionCalibrated`. | Kept the test-only policy and override. Existing lifecycle, startup, lease, and timing tests select the mock wait policy for their authored test timing; changing it would alter test wait behavior. It is not part of production wait configuration. | `engine/tests/support.rs`, Desktop mock helpers, `engine/config/effective_policy.rs`, and feature-gated test caller census; both Rust suites. |
| Schema-25 worker/snapshot overlap: `WorkerMetricsLocal.elapsed_us`, `EngineSnapshot.elapsed_us`, session clock projection | The worker metrics type retained an unused legacy `elapsed_us` copy. It had no worker writer or snapshot consumer; tests alone assigned it. Public session snapshots already derive elapsed progress from the shared session-clock projection. | Removed the unused worker-local field and its merge assertions. Kept the public snapshot projection, `total_us` worker accounting, schema version 25, serialized key sets, exact output values, QPC provenance, and missing-QPC semantics unchanged. No `RtTraceRecord` or `NativeTelemetryOutput` field was deduplicated. | `native_telemetry_schema_25_serialized_key_sets_are_pinned`; schema-25 missing-completion/QPC tests; snapshot and telemetry suites. |

## Behavioral and compatibility evidence

- `prepared_plan_rounds_qpc_offsets_up_and_checkpoints_validation` uses
  3,579,545 Hz (not divisible by 1,000,000) and checks H/G at one tick below,
  exactly at, and one tick above each boundary: G at 225/226/227 and H at
  240/241/242. The prepared and raw executor decision oracle remains in the
  core test only; no raw executor is present at the native worker boundary.
- `prepared_schedule_cancellation_completes_before_session_construction`
  cancels the same cancellable preparation routine before any native session
  or worker exists. Existing core tests continue to cover plan overflow,
  capacity, and cancellation.
- The full engine/core suites retain mixed Up/Down, 1/5/15-key chord,
  one-attempt, owner final-gate, release/quarantine, raw-zero/partial/QPC,
  focus recovery, and no-allocation regressions. These are mock or scripted
  sender/QPC tests where applicable.
- `NativeTelemetryOutput` remains schema 25. The removed worker-local field
  was not serialized. Compatibility elapsed time continues to come from the
  session clock projection; no missing timestamp is reconstructed from a
  target or another QPC sample.
- No Windows ReceiveOnly run, real game playback, SendInput performance
  qualification, or M3 work was performed. Build/test results establish code
  and mock/scripted behavior only, not native timing or game reception.

## Local verification

All commands below ran against the final local source before commit.

| Working directory | Command | Result |
| --- | --- | --- |
| `rust/` | `cargo check --locked -p sky_dispatch_core -p sky_dispatch_win32 -p sky_player` | PASS; default check, no warnings. |
| `rust/` | `cargo check --locked -p sky_dispatch_core -p sky_dispatch_win32 -p sky_player --all-features` | PASS; all-feature check, no warnings. |
| `rust/` | `cargo test --locked -p sky_dispatch_core -p sky_dispatch_win32 -p sky_player --quiet` | PASS; all default targets passed. Visible summaries included core 87/87, Win32 278 passed/1 ignored, and `sky_player` 275 passed/1 ignored. |
| `rust/` | `cargo test --locked -p sky_dispatch_core -p sky_dispatch_win32 -p sky_player --all-features --quiet` | PASS; core 87/87, Win32 278 passed/1 ignored, `sky_player` library 301 passed/1 ignored, acceptance CLI 52/52, and no-allocation integration 4/4. Other listed binary/integration targets also passed. |
| `rust/` | `cargo clippy --locked -p sky_dispatch_core -p sky_dispatch_win32 -p sky_player --all-targets --all-features -- -D warnings` | PASS; no warnings. |
| `rust/` | `cargo fmt --all -- --check` | PASS. |
| repository root | `cargo xtask check desktop-native` | PASS; desktop runtime default/all-feature checks and `tauri-test` library tests completed. |
| repository root | `git diff --check` | PASS. |

No PowerShell or TypeScript files changed, so their script/tooling checks were
not run. No `cargo xtask check all`, ReceiveOnly run, native performance
qualification, or M3 work was run.
