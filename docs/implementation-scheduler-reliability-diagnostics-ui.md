# Scheduler reliability + Diagnostics UI implementation handoff

## Status

Implementation handoff for Codex.

- Repository: `pumni/Sky-Auto-Player`
- Audited base commit: `0a6da845e7354cb785c2a14d1bfcbd7b998f9fcf`
- Handoff branch: `docs/scheduler-vnext-handoff-2026-09`
- Target platform: Windows 11 x86_64
- Rust workspace policy: Edition 2024, MSRV 1.98
- Production input boundary: Windows `SendInput` only

This plan is intentionally code-oriented. Before editing, Codex must read `AGENTS.md`, `SECURITY.md`, the production files named below, and the direct tests for each changed boundary.

## Objective

Improve playback reliability and make the reason for every sender-side missed note diagnosable from the app UI without making the real-time dispatch path materially heavier.

The work must preserve the current architecture:

```text
song
  -> materialized timing policy
  -> compiled dispatch intents
  -> RuntimeDispatchCoordinator
  -> immutable NextDispatchPlan / PreparedPhysicalPacket
  -> high-resolution wait + bounded spin
  -> final control / target / focus / lease gates
  -> final QPC sample
  -> Down deadline cutoff
  -> one SendInput call
  -> commit or recovery
  -> fixed telemetry
  -> snapshot
  -> Tauri diagnostics.snapshot
  -> React Diagnostics UI
```

## Non-negotiable invariants

1. Do not introduce a second gameplay input mechanism. `SendInput` remains the only physical input boundary.
2. Do not add hooks, game-memory access, DLL injection, debugger attachment, or anti-cheat bypass behavior.
3. Do not move JSON, formatting, logging, allocation-heavy work, async runtimes, or UI work into the real-time worker.
4. Do not retry partial Down-bearing or Mixed authored packets. Partial Down/Mixed remains fail-closed.
5. Chords remain a single prepared physical packet / single `SendInput` call.
6. Final exact HWND/focus validation remains immediately before Down-bearing physical dispatch.
7. Supervisor lease admission remains authoritative.
8. Metrics written on the dispatch thread must be scalar/fixed-capacity and nonblocking.
9. Production Diagnostics must remain observational. Enabling the UI must not switch the worker to `StrictTimingDiagnostic` or feed telemetry back into scheduling.
10. Generated TypeScript bindings under `desktop/src/bridge/generated/` must not be hand-edited. Regenerate with `cargo xtask bindings generate`.

---

# Workstream A — toolchain hygiene

## A1. Pin Rust 1.98.1

Files:

- `rust/rust-toolchain.toml`

Change:

```diff
-channel = "1.98.0"
+channel = "1.98.1"
```

Keep:

```toml
rust-version = "1.98"
```

Do not change Edition, panic strategy, LTO, codegen units, or target CPU as part of this task.

Acceptance:

- `cargo xtask check static`
- `cargo xtask check rust`
- build metadata reports the expected compiler version.

Suggested commit:

`build: pin production toolchain to rust 1.98.1`

---

# Workstream B — close the 500–2000 us diagnostics blind spot

The production Down cutoff can reject a note after approximately 500 us of pre-call lateness, while existing coarse counters begin at 2 ms. The UI can therefore look clean while Down events are already being suppressed.

This is the highest-priority instrumentation change.

## B1. Add fixed pre-call lateness buckets

Primary files:

- `rust/crates/sky_player/src/engine/telemetry/metrics.rs`
- `rust/crates/sky_player/src/engine/worker/health.rs`
- `rust/crates/sky_player/src/engine/session.rs`
- `rust/crates/sky_player/src/engine/snapshot.rs`
- direct telemetry/engine tests
- `rust/crates/sky_player/tests/rt_dispatch_no_alloc.rs`

Add cumulative, mutually-exclusive, session-scoped counters to `WorkerMetricsLocal` and the published snapshot:

```rust
pre_call_lt_250us: u64,
pre_call_250_500us: u64,
pre_call_500_750us: u64,
pre_call_750_1000us: u64,
pre_call_1000_1500us: u64,
pre_call_1500_2000us: u64,
pre_call_ge_2000us: u64,
```

Bucket semantics must be explicit and tested:

- `[0, 250)`
- `[250, 500)`
- `[500, 750)`
- `[750, 1000)`
- `[1000, 1500)`
- `[1500, 2000)`
- `[2000, +inf)`

Record exactly one bucket for every production pre-call lateness observation for which a valid pre-call QPC exists.

Do not allocate. Do not use a `Vec`, map, mutex, formatted string, or histogram library on the dispatch path.

Keep the existing 2/5/10 ms counters for compatibility and long-tail visibility.

### Required tests

- exact boundary tests at 0, 249, 250, 499, 500, 749, 750, 999, 1000, 1499, 1500, 1999, 2000 us;
- saturation behavior;
- one observation increments exactly one fine-grained bucket;
- existing 2/5/10 ms counters still behave identically;
- update `rt_dispatch_no_alloc.rs` so the added accounting is covered by an allocation-free hot-path test.

Suggested commit:

`feat(player): record sub-2ms pre-call lateness buckets`

## B2. Surface existing failure taxonomy instead of inventing duplicate counters

The engine already has useful counters. Reuse them.

Expose through the lightweight production snapshot and Diagnostics projection at least:

```text
missed_down_boundaries
missed_down_keys
missed_backlog_boundaries
missed_hard_late_boundaries
final_gate_cutoff_misses
final_gate_control_rejections
final_gate_target_changes
final_gate_focus_losses
final_gate_lease_expirations
sendinput_partial_events
sendinput_zero_progress_failures
```

If `last_error` is already available from the snapshot, project it as an optional bounded Diagnostics field rather than creating a second error store.

Do not reinterpret these counters:

- `missed_hard_late_boundaries` is musical recovery classification.
- `final_gate_cutoff_misses` is final physical admission/cutoff evidence.
- `final_gate_focus_losses`, target changes and lease expirations identify separate suppression causes.
- `sendinput_partial_events` and `sendinput_zero_progress_failures` are transport evidence, not scheduler lateness.

The UI must preserve these distinctions.

Suggested commit:

`feat(player): publish dispatch suppression taxonomy`

---

# Workstream C — carry Diagnostics data to the Tauri contract correctly

## C1. Extend the existing Diagnostics DTO

Files:

- `desktop/src-tauri/src/native_runtime.rs`
- `desktop/src-tauri/src/ui_events.rs`
- generated binding output after regeneration

Do not create a second diagnostics event. Continue using:

`diagnostics.snapshot`

Extend `NativeDiagnosticsSample` and `DiagnosticsSnapshotDto` with the fine buckets and failure-taxonomy counters from Workstream B.

Also include the session's effective Down grace:

```rust
down_late_grace_us: u64
```

This value must be the actual immutable value used by the active worker, not a UI default copied from settings.

If the active player/backend snapshot is unavailable, preserve the existing availability semantics. Do not turn unavailable production evidence into numeric zero.

### Optional bounded field

If plumbing remains straightforward, include:

```rust
last_error: Option<String>
```

Validate it using the same bounded text contract already used in `ui_events.rs`.

## C2. Keep DTO validation bounded

Update `UiEvent::validate_diagnostics_snapshot` for any new value that needs an explicit bound.

Counters may be monotonic `u64`; duration/config values must receive a sensible upper bound.

Do not weaken `#[serde(deny_unknown_fields)]`.

## C3. Regenerate TypeScript bindings

Run:

```powershell
cargo xtask bindings generate
```

Expected generated change includes:

- `desktop/src/bridge/generated/DiagnosticsSnapshotDto.ts`

Do not edit that file manually.

Suggested commit:

`feat(desktop): extend diagnostics snapshot contract`

---

# Workstream D — Diagnostics UI must show the real sender-side failure reason

This is a required product outcome, not optional polish.

Primary files:

- `desktop/src/components/utility/DiagnosticsView.tsx`
- `desktop/src/components/utility/DiagnosticsView.test.tsx`
- `desktop/src/state/store.ts`
- `desktop/src/state/store.test.ts`
- `desktop/src/bridge/mockBridge.ts`
- `desktop/src/styles/workbench.css`
- generated Diagnostics type

## D1. Performance tab layout

Keep existing completion metrics and their unavailable state. Add explicit groups.

### Timing

Keep:

- Completion p50
- Completion p95
- Session max
- Max pre-call lateness
- Completion jitter

Do not display observer-only completion metrics as zero when Production does not publish them.

### Pre-call distribution

Show the cumulative fine-grained buckets:

```text
< 250 us
250–500 us
500–750 us
750–1000 us
1.0–1.5 ms
1.5–2.0 ms
>= 2.0 ms
```

Also show:

`Down cutoff grace: N us`

The user must be able to see immediately how much pre-call traffic is landing beyond the configured Down grace.

### Deadline admission

Show:

- Hard-late Down boundaries
- Missed Down keys
- Backlog boundaries
- Final cutoff misses
- Focus gate rejections
- Target changes
- Lease expirations
- Control rejections

Use short tooltips/help text if necessary, but do not collapse these into one generic "Dropped" value.

### Input transport

Show:

- SendInput zero-progress failures
- SendInput partial events
- Dropped keys
- Chord splits
- Stuck keys
- Active keys

This group answers "did Windows input transport fail?" independently of deadline admission.

### Release

Keep current release metrics.

## D2. Timing tab: make it useful in Production

The current plot is based on completion p95, which can legitimately be unavailable in Production because the deferred diagnostic observer is not enabled.

Change the production-default plot to a sender-side metric that Production always has when backend metrics are available:

`max_sendinput_pre_call_lateness_us`

Recommended behavior:

- plot recent per-snapshot session max pre-call lateness in microseconds;
- draw a labeled horizontal threshold at `down_late_grace_us`;
- preserve completion p95 as optional secondary information only when available;
- if the backend is unavailable, show the current explicit unavailable state rather than an empty zero plot.

The graph must make a 600–900 us problem visually obvious when the cutoff is 500 us.

Do not enable the strict observer merely to make the graph populate.

## D3. Failure summary

At the top of the Performance tab, add a compact sender-side summary derived from current cumulative counters.

Example states:

```text
Sender-side status: Healthy
No Down suppression recorded this session.
```

or:

```text
Sender-side status: Attention
14 hard-late boundaries; 2 focus rejections; 0 SendInput transport failures.
```

This is a UI derivation only. Do not feed it back into Rust scheduling and do not redefine the backend health state.

## D4. Events tab

Do not emit one UI event per note or per timing bucket.

`diagnostics.snapshot` remains coalescible/latest-wins telemetry and should continue to be excluded from the human event log.

The Events tab may continue showing lifecycle/failure events. If discrete diagnostic events are added later, only emit state transitions or terminal conditions, not every sample.

## D5. Mock and store behavior

Update `mockBridge.ts` so its `diagnostics.snapshot` fixture contains deterministic nonzero examples for:

- one fine lateness bucket;
- one hard-late count;
- one focus/target gate count;
- zero-progress/partial transport counters;
- `down_late_grace_us`.

Update store tests so:

- a new session starts a fresh Diagnostics sample history;
- stale session samples do not appear as active;
- new fields survive event ingestion unchanged;
- unavailable observer metrics remain unavailable rather than zero.

### Required UI tests

Extend `DiagnosticsView.test.tsx` to assert:

1. the 500–750 us bucket is visible;
2. effective Down grace is visible;
3. hard-late, focus, target, lease and transport counters appear under distinct labels;
4. unavailable observer metrics stay `Unavailable`;
5. production backend metrics remain visible when completion distribution is unavailable;
6. Timing tab uses pre-call data without requiring completion p95;
7. no "0" is fabricated when backend status is unavailable;
8. accessibility names for the timing graph and groups remain meaningful.

Suggested commit:

`feat(desktop): visualize dispatch cutoff and suppression diagnostics`

---

# Workstream E — session-frozen Down grace plumbing

Do this after Workstreams B–D so behavior changes can be measured correctly.

Primary files to inspect:

- `rust/crates/sky_app_core/src/timing.rs`
- `rust/crates/sky_dispatch_core/src/compile.rs`
- `rust/crates/sky_player/src/engine/config.rs`
- `rust/crates/sky_player/src/engine/session.rs`
- `desktop/src-tauri/src/native_runtime.rs`
- schedule validation tests
- cutoff tests
- native acceptance harness

## E1. Preserve one timing policy for the entire session

The effective Down grace must be materialized before schedule construction and remain immutable for the active session.

Do not change grace per note.

Do not build an online adaptive controller.

Preferred conceptual model:

```rust
enum SessionReliabilityClass {
    Strict,    // 500 us
    Balanced,  // 750 us
    Resilient, // 1000 us
}
```

However, do not add a user-facing settings control unless it fits the existing settings/product contract cleanly. Internal policy plumbing and acceptance overrides are sufficient for this task.

## E2. Keep schedule constraints consistent

The same effective grace must participate in the timing policy used to compute/validate:

- minimum hold duration;
- minimum release gap;
- physical Down cutoff.

Never create a state where the worker uses a larger grace than the schedule validator assumed.

## E3. Behavior rollout

Initial implementation should preserve the current production default of 500 us while enabling deterministic A/B runs at 750 and 1000 us.

Do not silently change the shipped default until the real physical acceptance A/B in Workstream H has evidence.

Expose the effective value in Diagnostics as required by Workstream C.

### Tests

- frozen policy survives from preparation to worker config;
- schedule validation uses the same value;
- cutoff equality semantics are preserved: exactly at the allowed boundary remains allowed if that is the current contract; beyond it is rejected;
- test 500/750/1000 us with deterministic QPC seams.

Suggested commit:

`refactor(timing): freeze Down grace in session timing policy`

---

# Workstream F — detect likely UIPI blockage before playback

A common "sends nothing" case is a higher-integrity target process.

Add a control-plane integrity check; do not put token queries on the realtime dispatch thread.

Suggested ownership:

- add a small Win32 integrity helper in `sky_dispatch_win32` (new module if needed);
- call it from `desktop/src-tauri/src/native_runtime.rs` after target HWND discovery and before arming the player.

Implementation outline:

1. derive target PID from HWND with `GetWindowThreadProcessId`;
2. open target process with the minimum query right;
3. query current-process and target-process token integrity level;
4. compare integrity RIDs;
5. if target integrity is strictly higher than the player, fail startup with a precise user-facing error explaining that Windows UIPI can block `SendInput`;
6. if integrity cannot be determined for a benign environmental reason, return an explicit Unknown result and preserve current startup behavior rather than guessing.

Do not claim `GetLastError` from a zero-return `SendInput` can identify UIPI; it cannot reliably do so.

Tests should isolate integrity comparison logic from Win32 handle acquisition.

If straightforward, expose compatibility in Diagnostics/environment information as:

`compatible | mismatch | unknown`

but do not delay the core startup preflight for extra UI work.

Suggested commit:

`feat(win32): preflight target integrity for SendInput UIPI`

---

# Workstream G — isolate supervisor heartbeat from UI publication stalls

Inspect:

- `desktop/src-tauri/src/native_runtime.rs`

The current monitor loop owns foreground sampling, heartbeat, polling, snapshot publication and Diagnostics/UI publication. A multi-second block in publication should not starve the realtime supervisor lease.

Required outcome:

- heartbeat progression must not depend on a potentially blocking UI channel/event send;
- do not move UI work to the realtime worker;
- keep shutdown/join ownership deterministic;
- do not increase heartbeat frequency merely to mask coupling.

A minimal dedicated supervisor-heartbeat task/thread is preferable to a broad monitor rewrite.

Tests need a seam that can intentionally stall UI publication while proving heartbeat continues.

Suggested commit:

`refactor(desktop): isolate playback supervisor heartbeat`

---

# Workstream H — physical Windows acceptance and A/B gate

The current code has strong deterministic/mock coverage, but the reliability change must be checked through real `SendInput` on an interactive Windows desktop.

Use or extend:

- `rust/crates/sky_player/src/bin/rt_native_acceptance.rs`
- existing native acceptance docs/scripts
- controlled foreground sink infrastructure if already present; otherwise add the smallest repository-owned sink required for acceptance

Required scenarios:

1. single notes;
2. full chords;
3. holds;
4. rapid same-key retrigger;
5. mixed Up/Down packet;
6. focus loss;
7. target HWND change;
8. pause/resume;
9. skip/stop cleanup;
10. all-up verification;
11. partial/zero-progress fault seams where real injection cannot deterministically produce them;
12. 500 vs 750 vs 1000 us grace A/B.

Capture at minimum:

```text
max_sendinput_pre_call_lateness_us
fine pre-call buckets
missed_hard_late_boundaries
missed_backlog_boundaries
final_gate_cutoff_misses
final_gate_focus_losses
final_gate_target_changes
final_gate_lease_expirations
sendinput_partial_events
sendinput_zero_progress_failures
stuck/possibly-active/failed-release state
```

## Default-change gate

Only change the production default from 500 us after the controlled physical A/B shows:

- a meaningful reduction in hard-late dropped Down boundaries;
- no unacceptable hold/retrigger regression;
- no new stuck-key/cleanup regression;
- no chord integrity regression.

If 750 us captures most of the 500–1000 us tail, prefer 750 over 1000. Choose the smallest grace supported by evidence.

If the physical sink cannot be run on the laptop, keep the production default at 500 us and report the missing evidence. Do not infer a new default from mock timing alone.

---

# Workstream I — verification

During development run narrow checks first:

```powershell
cargo xtask check static
cargo xtask check rust
cargo xtask bindings generate
cargo xtask check desktop
```

Before handoff completion:

```powershell
cargo xtask check all
```

Also run the specialized native acceptance path required by Workstream H on an interactive Windows session.

For every specialized test that cannot be run, record:

- exact command not run;
- why it could not run;
- which behavior remains unqualified.

---

# Expected file touch map

The implementation should be centered on these files; Codex should avoid unrelated churn.

## Rust realtime / telemetry

- `rust/crates/sky_player/src/engine/telemetry/metrics.rs`
- `rust/crates/sky_player/src/engine/worker/health.rs`
- `rust/crates/sky_player/src/engine/worker/dispatch/authored.rs`
- `rust/crates/sky_player/src/engine/worker/dispatch/recovery.rs` only if existing counters need propagation, not duplicate accounting
- `rust/crates/sky_player/src/engine/session.rs`
- `rust/crates/sky_player/src/engine/snapshot.rs`
- `rust/crates/sky_player/tests/rt_dispatch_no_alloc.rs`

## Timing policy

- `rust/crates/sky_app_core/src/timing.rs`
- `rust/crates/sky_player/src/engine/config.rs`
- relevant compile/validation/cutoff tests

## Win32

- `rust/crates/sky_dispatch_win32/src/lib.rs`
- new integrity helper or the narrow existing module that owns process/HWND Win32 queries

## Desktop Rust

- `desktop/src-tauri/src/native_runtime.rs`
- `desktop/src-tauri/src/ui_events.rs`

## React / TypeScript

- `desktop/src/components/utility/DiagnosticsView.tsx`
- `desktop/src/components/utility/DiagnosticsView.test.tsx`
- `desktop/src/state/store.ts`
- `desktop/src/state/store.test.ts`
- `desktop/src/bridge/mockBridge.ts`
- `desktop/src/styles/workbench.css`
- regenerated `desktop/src/bridge/generated/DiagnosticsSnapshotDto.ts`

## Toolchain

- `rust/rust-toolchain.toml`

---

# UI data contract reference

All new Diagnostics values are session-scoped cumulative values unless explicitly stated otherwise.

Recommended DTO additions:

```text
down_late_grace_us

pre_call_lt_250us
pre_call_250_500us
pre_call_500_750us
pre_call_750_1000us
pre_call_1000_1500us
pre_call_1500_2000us
pre_call_ge_2000us

missed_down_boundaries
missed_down_keys
missed_backlog_boundaries
missed_hard_late_boundaries

final_gate_cutoff_misses
final_gate_control_rejections
final_gate_target_changes
final_gate_focus_losses
final_gate_lease_expirations

sendinput_partial_events
sendinput_zero_progress_failures

last_error (optional)
```

Keep existing:

```text
max_lateness_us
p50_ms
p95_ms
sigma_onset_ms
late_2ms
late_5ms
late_10ms
max_sendinput_pre_call_lateness_us
pre_call_late_2ms
pre_call_late_5ms
pre_call_late_10ms
active_keys
stuck_keys
keys_dropped
chord_split_events
backend_status
release_max_us
release_late_2ms
session_id
```

Do not remove or rename existing fields in this implementation unless a direct test proves the old field is unused and the schema change is intentionally coordinated.

---

# Diagnostics interpretation contract

The UI should let an engineer classify a missed note using this decision tree:

```text
hard-late / cutoff counter increased?
  yes -> scheduler reached final boundary too late; inspect pre-call buckets
  no
    focus or target counter increased?
      yes -> intentional safety suppression
      no
        lease/control rejection increased?
          yes -> orchestration/control-plane suppression
          no
            SendInput zero-progress/partial increased?
              yes -> Windows input transport/integrity path
              no
                sender-side evidence clean -> investigate game-side input visibility / hold duration
```

This decision tree is the product requirement for the Diagnostics UI.

---

# Explicit non-goals

Do not implement in this task:

- per-note adaptive grace;
- PID/EWMA/ML timing controller;
- Tokio/async dispatch;
- CPU affinity pinning;
- `TIME_CRITICAL` or `Pro Audio` as the default priority;
- global `timeBeginPeriod` dependency;
- direct DPC/ISR monitoring inside the app;
- game input hooks or game-memory observation;
- retries for partial Down/Mixed packets;
- per-note UI events/logging.

For OS scheduling investigations, external WPR/WPA traces remain the preferred evidence source.

---

# Recommended commit order for Codex

1. `build: pin production toolchain to rust 1.98.1`
2. `feat(player): record sub-2ms pre-call lateness buckets`
3. `feat(player): publish dispatch suppression taxonomy`
4. `feat(desktop): extend diagnostics snapshot contract`
5. `feat(desktop): visualize dispatch cutoff and suppression diagnostics`
6. `refactor(timing): freeze Down grace in session timing policy`
7. `feat(win32): preflight target integrity for SendInput UIPI`
8. `refactor(desktop): isolate playback supervisor heartbeat`
9. `test(native): qualify physical dispatch grace variants`

Each commit should compile and carry its direct tests where practical. Do not leave a multi-commit interval where generated bindings and source DTO definitions disagree.

---

# Codex completion report

When finished, Codex should return a concise report containing:

1. commits created;
2. exact files changed;
3. final effective production Down grace;
4. Diagnostics fields added;
5. screenshot or textual confirmation of the new Diagnostics UI sections;
6. Rust no-allocation test result;
7. `cargo xtask check all` result;
8. physical native acceptance result for 500/750/1000 us;
9. whether UIPI mismatch was tested;
10. any test that could not be run and residual risk.

The implementation is complete only when the UI can distinguish at least these sender-side outcomes without reading logs:

- hard-late/cutoff suppression;
- focus loss;
- target change;
- supervisor lease/control rejection;
- SendInput zero progress;
- SendInput partial insertion;
- no sender-side failure detected.

