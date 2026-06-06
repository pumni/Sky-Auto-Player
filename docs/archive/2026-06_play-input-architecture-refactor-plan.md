> ARCHIVED 2026-06 — historical plan/audit. Không phải tài liệu hiện hành.
> Contract & sự thật hiện tại: ../timing-principles.md và ../architecture.md.
> CẢNH BÁO lệch code đã biết: kế hoạch refactor play/input pipeline cũ.

# Play/Input Architecture Refactor Plan

Status: Phase A/B completed, Phase C.1 completed, and Phase D.1 started on 2026-06-06.

Goal: make the playback/input pipeline easier to reason about, test, and diagnose. The refactor must
preserve the hard project constraints: no game-file modification, no game-memory reading, no
anti-cheat bypass, and Windows SendInput only.

## 1. Problem Statement

The current player can produce clean-looking sender telemetry while the game still misses notes. This
is expected at the boundary: telemetry can prove that the player attempted a SendInput event, but it
cannot prove that the game sampled the key-down state.

There are also architecture issues in the current threaded playback path:

- The input backend is not owned by only one thread because HUD rendering can call
  `backend.get_health()`.
- The main/render thread can still disturb the dispatch thread through Python's GIL.
- Existing threaded tests use `time.sleep()` for slow-render simulation, which releases the GIL and
  misses CPU-bound interference.
- Telemetry mixes several concerns but does not cleanly separate intended schedule, runtime dispatch,
  SendInput acceptance, and game-observed audio.
- Game FPS toggling is not reliable enough to be a supported workaround.

## 2. Target Architecture

The target system has five explicit layers.

1. **Song and Scheduler Layer**
   - Pure Python, deterministic, unit-testable.
   - Converts parsed notes into `KeyAction`/intent timelines.
   - No Win32 calls, no clocks, no sleeping, no renderer.

2. **Runtime Plan Layer**
   - Converts scheduled actions into per-key generations and release obligations.
   - Owns same-key feasibility, stale-up suppression, and runtime drop accounting.
   - Pure or near-pure, testable with fake clocks/backends.

3. **Realtime Dispatch Layer**
   - The only layer that can call `backend.key_down`, `backend.key_up`, `backend.release_all`, and
     `backend.get_health`.
   - Owns deadline waiting, command handling, focus pause state, telemetry recording, and final
     release.
   - Runs on the dispatch thread in production.

4. **Control/UI Layer**
   - Polls hotkeys, focus state, and renders HUD.
   - Never touches the input backend directly.
   - Receives immutable snapshots from the dispatch layer.

5. **Evidence Layer**
   - Stores sender telemetry, backend health snapshots, and optional audio/onset analysis.
   - Explicitly labels which evidence is pre-send, SendInput-side, and after-send/game-observed.

## 3. Invariants

These invariants should be written as tests.

- Only the dispatch owner calls any `InputBackend` method.
- The HUD never reads backend state directly; it renders a `BackendHealth` snapshot provided by the
  dispatch owner.
- CPU-bound renderer work cannot delay dispatch deadlines beyond a defined tolerance.
- Slow focus checks cannot delay dispatch deadlines.
- Command handling for pause, panic, skip, and quit is serialized through the dispatch owner.
- Telemetry must distinguish:
  - intended down count;
  - sent down count;
  - runtime-dropped down count;
  - backend-skipped down count;
  - game-heard onset count when audio evidence is provided.
- A clean sender run must never be described as proof of game acceptance.

## 4. Refactor Phases

### Phase A - Freeze The Current Behavior With Tests

Add tests before changing code.

- Backend owner test that includes the real HUD behavior. The test should fail if
  `ProgressRenderer.render()` calls `backend.get_health()` from the control thread.
- CPU-bound UI isolation test. The renderer should busy-loop for 1-10 ms, not `time.sleep()`, so the
  test exercises GIL interference.
- Focus isolation test with a focus guard that performs CPU-heavy work and Win32-like blocking.
- Telemetry vocabulary test: a fully sent run, an internally dropped run, and a backend-skipped run
  must produce clearly different summaries.

Acceptance:

- The tests expose the current architecture issues without changing production behavior.

### Phase B - Snapshot-Only HUD

Move backend health reads out of `ProgressRenderer`.

- The dispatch owner samples `backend.get_health()` at controlled points.
- Progress snapshots include immutable fields such as active key count, failed release count, last
  backend error, input-path degraded flag, and recent timing counters.
- `ProgressRenderer` becomes a pure renderer of snapshots and no longer receives or stores backend.

Acceptance:

- No UI/control code calls `InputBackend`.
- Backend owner invariant passes with the real HUD.

### Phase C - Dispatch Thread Isolation Cleanup

Reduce GIL and control-loop interference.

- Keep the dispatch loop small: wait, poll command queue, read atomic/shared focus value, drain due,
  call backend, record telemetry.
- Ensure progress publication is bounded and uses simple immutable snapshots.
- Consider lowering render frequency during dense playback or making render coalescing stricter.
- Avoid CPU-heavy formatting on the dispatch thread.
- Review whether MMCSS/waitable-timer setup is effective and log fallback status explicitly.

Acceptance:

- CPU-bound renderer tests pass within a strict p95/p99 dispatch interval tolerance.
- Slow focus/control tests pass.

### Phase D - Timing Evidence Split

Restructure telemetry around evidence boundaries.

Suggested event fields:

- `schedule_id`, `generation_id`, `source_note_index`
- `scheduled_down_us`, `scheduled_up_us`
- `dispatch_start_us`, `dispatch_end_us`
- `sendinput_success`, `sendinput_error`
- `sent_scan_codes`, `skipped_scan_codes`
- `runtime_outcome`
- `backend_health_snapshot_id`

Suggested summaries:

- `sender_clean`: true only when intended downs equal sent downs and there are no runtime/backend
  drops.
- `game_acceptance_unknown`: true unless audio/onset evidence is attached.
- `after_send_missing_count`: only computed from audio/onset evidence.
- `input_path_health`: p50/p95/p99 send duration and sustained degraded intervals.

Acceptance:

- Reports cannot imply game acceptance without audio evidence.
- `measure_stutter.py` or a replacement can ingest telemetry and produce a clear before-send vs
  after-send verdict.

### Phase E - Controlled Probe Suite

Create explicit probe songs and procedures for diagnosing game sampling.

Recommended probes:

- Single-key metronome, wide spacing.
- Alternating-key metronome, wide spacing.
- Chord probe with exact simultaneous notes.
- Same-key repeat staircase.
- Hold sweep at fixed FPS: 5 ms through 20 ms.
- Phase sweep: fixed short hold with down offset stepped across one likely game tick.

Acceptance:

- A clean probe recording can answer whether a miss happened before SendInput or after SendInput.
- The project has a repeatable way to estimate minimum reliable hold without relying on FPS toggling.

### Phase F - Policy Layer For Reliability vs Sharpness

Only after evidence is clean, add a policy layer that can choose between precision and reliability.

Possible policies:

- `local-precise`: shortest hold that is known to work in the current environment.
- `stable-144`: minimal empirically reliable hold for 144 FPS, maybe slightly above one frame but
  below 60 FPS hold.
- `diagnostic`: runs probes and reports recommended hold/profile; does not auto-change behavior.

Acceptance:

- No global hold increase is made without probe evidence.
- The user can keep the sharp 144 FPS feel when the game/input state supports it.
- Safer settings are recommended with explicit evidence, not hidden heuristics.

## 5. Out Of Scope

- No PostMessage, driver, hook, HID emulation, game memory reads, or anti-cheat bypass.
- No broad scheduler rewrite until the ownership and evidence layers are clean.
- No FPS-toggle workaround as product behavior.
- No claim that SendInput success equals game acceptance.

## 6. Immediate Recommended Next Step

Phase A/B initial work completed:

- Added backend-owner coverage that records `get_health()` calls.
- Added real-HUD threaded ownership coverage.
- Added a short CPU-bound render interference test.
- Removed direct HUD/backend coupling by rendering backend health from dispatch-owned snapshots.

Phase C.1 initial work completed:

- Coalesced backend health reads behind a dispatch-owned snapshot cache instead of sampling health on
  every progress publish.
- Split threaded control polling and focus polling into explicit cadences so the main/control thread
  does less CPU-bound work while dispatch is active.
- Added CPU-bound focus and control polling tests that measure real backend down-call spacing.

Phase D.1 initial work completed:

- Added telemetry evidence-boundary summary fields for schedule, runtime dispatch, SendInput-side
  results, and game-observed evidence availability.
- Added `sender_clean`, `before_send_missing_down_count`, `game_acceptance_unknown`,
  `game_observed_onset_count`, and `after_send_missing_count` to telemetry summaries.
- Updated telemetry inspection output so a clean sender run is explicitly reported as not proving
  game acceptance without audio/onset evidence.
- Added tests that distinguish sender-clean runs from before-send missing-note runs.

Next step:

1. Build the controlled probe suite for after-SendInput/game-sampling evidence.
2. Wire optional audio/onset analysis into the same evidence-boundary summary shape.
3. Review remaining dispatch-thread hot paths for CPU-heavy formatting or logging.

The cleanup removes known backend ownership dirt and reduces main-thread GIL pressure without changing
the musical policy yet.
