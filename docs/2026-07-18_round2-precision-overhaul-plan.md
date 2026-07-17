# Plan: Round-2 precision overhaul — polled-wait patch, min_hold device margin, gap visibility, estimator simplification

> **Status:** APPROVED for implementation (2026-07-18). Author: deep-review round 2 (critical pass, evidence-backed).
> **For:** an AI refactor agent. Follow `AGENTS.md` exactly — P0 security mandates are immutable,
> use `uv run` for every Python execution, use harness tools (Read/Edit/Grep/Glob) for all file work.
> **Companion docs:** `docs/2026-07-18_spin-lead-audit-and-hotpath-optimization-plan.md` (Phases 1–3
> of that plan are ALREADY SHIPPED in the working tree — do not redo them),
> `docs/timing-principles.md` (must be updated by Phase 3 of THIS plan).
>
> **Human decisions already made (do not re-litigate):**
> 1. `onset_bias_us` is unnecessary → REMOVE it (Phase 5A).
> 2. The polled-wait CPU hole must be patched (Phase 1).
> 3. `min_hold` gets a constant additive margin, designed to become per-device later (Phases 3, 6).
> 4. Estimator simplification (retire RLS linear model + disk cache) is APPROVED (Phase 5B).
> 5. Do NOT chase sub-frame sender precision anywhere — the game frame-samples input; the sender
>    is already below the observable noise floor.

---

## 0. Ground rules for the executing agent

- **One phase = one commit**, in the order given (Phase 6 is a separate track / separate PR).
- **Read every file before editing it.** Line numbers below are from the 2026-07-18 working tree;
  re-locate by content if they have drifted.
- After each phase run the phase gate (see §8), then before the final commit of the sequence run
  the full triad: `uv run ruff check . && uv run pyright && uv run pytest`.
- **Golden/expected-value churn must be recomputed by formula, never regenerated blindly.**
  Phase 3 lists the exact arithmetic. If a test fails that is NOT listed as expected churn in its
  phase, STOP and investigate the root cause — do not force-update assertions.
- Keep diffs surgical. Do not refactor neighbouring code, do not reformat untouched lines.

### Evidence backing this plan (measured 2026-07-18, Win11, Python 3.14.5 free-threaded)

| Measurement | p50 | p90 | p99 | max |
|---|---|---|---|---|
| Thread spawn → thread entry | 139 µs | 192 µs | ~800 µs | 797 µs |
| Spawn → post MMCSS/priority scope (full dispatch prologue) | 165 µs | 224 µs | ~1 ms | 998 µs |
| High-res waitable-timer wake error (2 ms sleep, idle) | 281 µs | 544 µs | 744 µs | 797 µs |
| Same, with 3 CPU-burner threads | 384 µs | 529 µs | 799 µs | 799 µs |

Consequences: the 700 µs spin floor is correctly sized (do not lower it); the dispatch prologue is
sub-frame (epoch rebase is telemetry hygiene, not an audible fix); sender-side UP lateness only
*lengthens* holds — the only hold-shrinking risk is kernel delivery asymmetry after SendInput
returns, which motivates the Phase 3 margin.

---

## Phase 1 — Polled-wait CPU patch + polled-degradation telemetry flag

### 1.1 Raise the polled sleep cap from 1 ms to 2 ms

**Problem.** `HybridWaitStrategy.wait_until_us`, timer-aware polled ladder
(`src/sky_music/infrastructure/wait_strategy.py:104-109`): when there is no command event, sleeps
are capped at `1_000` µs per call so the dispatch loop can poll commands between steps. But the
loop only polls every `poll_interval_us = 2_000` (`src/sky_music/orchestration/core/loop.py:980`),
so half of those ~1000 wake-ups/second during long inter-note gaps are pure waste.

**Change.** In `wait_strategy.py`, the polled branch only:

```python
# BEFORE
sleep_us = min(remaining_to_sleep, 1_000)
# AFTER
sleep_us = min(remaining_to_sleep, 2_000)
```

Update the adjacent comment: the cap is aligned with the dispatch loop's `poll_interval_us`
(2 ms) — command acknowledgement latency is unchanged because the loop polls at that cadence
anyway.

**Do NOT touch:** the event-driven branch (`enable_event_wait` / `WaitForMultipleObjects`), the
`spin_until_us` implementation, the fallback (non-high-res) sleep ladder, or `spin_threshold_us`
semantics.

### 1.2 Telemetry flag when threaded dispatch degrades to polled waits

**Where.** `src/sky_music/orchestration/playback_supervisor.py:315-319` (`_run_threaded`): when
`self.enable_event_wait` is True but `inputs.create_auto_reset_event()` returned `None`, the run
silently burns polled wake-ups. This happens on the main thread BEFORE the dispatch thread starts,
so writing telemetry here is race-free.

**Change.** Immediately after the existing `debug_log("[realtime] command event unavailable...")`
line, add:

```python
self.telemetry.record_runtime_options(
    {
        **self.telemetry.runtime_options,
        "event_wait_degraded_to_polled": True,
    }
)
```

Do NOT set the flag when `enable_event_wait` is False (polled mode is then deliberate) and do not
add an `else` writing `False` — absence means "not degraded" (keeps summary keys stable, invariant
I9 style).

### 1.3 Expected test churn

- Run `uv run pytest -k "wait or strategy or spin or dispatch"` — no listed churn is expected.
  If a test hard-codes the 1 ms cap, update it to 2 ms and note it in the commit message.
- Check `tests/golden_schedules/telemetry_summary_schema_v1.json`: if `runtime_options` keys are
  enumerated/validated there, register `event_wait_degraded_to_polled` as OPTIONAL. If the schema
  treats `runtime_options` as freeform, no change.

**Commit:** `perf(wait): align polled sleep cap with 2ms command poll; flag polled degradation in telemetry`

---

## Phase 2 — Epoch-rebase default alignment (correction of an earlier finding)

**Reality check (verified):** epoch rebase is ALREADY ON in production. `RuntimeState`
(`src/sky_music/orchestration/runtime_session.py:40`) defaults `enable_epoch_rebase = True`;
`main.py:486` wires `--no-epoch-rebase`; `console_playback.py:610` and
`ui/textual_app/app.py:890,904,936` pass it through. Only the low-level constructor defaults are
`False`:

- `PlaybackEngine.__init__` — `src/sky_music/orchestration/engine.py:478`
- `PlaybackSupervisor.__init__` — `src/sky_music/orchestration/playback_supervisor.py:240`

That mismatch means an engine constructed without the explicit flag (legacy paths, tests, future
callers) silently behaves differently from production.

### 2.1 Change

Flip BOTH constructor defaults to `True`. Nothing else. The rebase itself only executes inside
`_run_threaded.dispatch_target` (`playback_supervisor.py:367-377`) as the final pre-run statement;
direct mode (`_run_direct`) never rebases regardless of the flag, so fake-clock/golden tests that
run direct mode are unaffected except for the recorded option value below.

### 2.2 Expected test churn (exactly one test)

`tests/test_threaded_dispatch.py:294` —
`test_threaded_epoch_rebase_defaults_off_in_engine_runtime_options` asserts
`runtime_options["epoch_rebase"] is False` and `"epoch_rebase_us" not in ...`. Update it to the
new contract:

- rename to `test_threaded_epoch_rebase_defaults_on_in_engine_runtime_options`;
- assert `runtime_options["epoch_rebase"] is True`;
- KEEP asserting `"epoch_rebase_us" not in summary["runtime_options"]` **if** the test runs in
  direct mode (no dispatch thread) — direct mode never rebases, so no measured delta is recorded.
  Read the test body first to confirm which mode it drives; adjust the assertion to match the mode
  it actually uses.

The other two rebase tests (`:313`, `:350`) pass `enable_epoch_rebase=True` explicitly and must
not change.

**Commit:** `fix(engine): align epoch_rebase constructor defaults with production-on runtime state`

---

## Phase 3 — `min_hold_margin_us`: constant device-latency margin on the frame model

> **Why.** The completion anchor measures hold sender-side (SendInput-return to SendInput-return).
> The game measures delivery-to-delivery. Residual kernel delivery latency after SendInput returns
> is acknowledged in-code as "generally <0.5 ms" and NOT accounted for
> (`engine.py:442-444`, `coordinator.py:296-299`). `balanced`'s ratio margin (1.02 → ~139 µs at
> 144 fps) is thinner than that residual; `local_precise` (1.0) has zero margin. A constant
> additive margin is the honest fix; Phase 6 later makes it measured per-device.

### 3.1 Semantics (read carefully — this is where mis-execution is most likely)

Let `margin = min_hold_margin_us` (µs, int, ≥ 0, **default 500**).

1. Margin applies **only in the frame-model branch** of
   `FrameTimingPolicy.from_timing_policy` (`src/sky_music/domain/scheduler_types.py:188-207`,
   the `fps is not None and fps > 0` branch):

   ```
   eff_hold_us     = ceil(hold_frames     × frame_us) + margin
   eff_min_hold_us = ceil(min_hold_frames × frame_us) + margin
   ```

   (`materialise_frame_us` already rounds; add margin AFTER it, do not fold margin into the
   frames ratio.)
2. Margin applies to **BOTH** `hold_us` and `min_hold_us`. Rationale: built-ins derive hold from
   min_hold (equal frames), and validation enforces `min_hold_us <= hold_us`
   (`domain/validation.py:248-250`). Adding margin to min_hold only would violate that invariant.
3. **Explicit overrides win verbatim, without margin:** if `policy.hold_override_us` /
   `policy.min_hold_override_us` is not None, the override value replaces the margin-included
   value exactly as today (the override assignment already comes after materialisation — keep that
   order; margin must be added BEFORE the override check so overrides fully win).
4. The unframed fallback branch (`fps` unknown/0 → `*_unframed_us`) gets **NO margin** — those
   values (17–22 ms) already carry ample slack.
5. `min_hold_margin_us = 0` must reproduce today's behaviour bit-for-bit (escape hatch and the
   regression-test lever).

### 3.2 Plumbing

- `TimingPolicy` (`scheduler_types.py:26`): add field
  `min_hold_margin_us: Microseconds = Microseconds(500)`; parse in `from_dict` via
  `max(0, int_value("min_hold_margin_us", 500))`.
- `FrameTimingPolicy` (`scheduler_types.py:156`): add the same field (carried value, used by
  `from_timing_policy` as described; keep it on the dataclass so telemetry/diagnostics can read
  what was applied).
- `DEFAULT_TIMING_PROFILES` (`src/sky_music/config.py:80-103`): do NOT add the key to the three
  built-in dicts — they inherit the 500 default from `from_dict`. A user profile may set it
  (including 0) to override.
- Enumerated-key surfaces: `grep -n "spin_threshold_us" src/` and mirror every list that
  enumerates known profile keys (verified hit: `src/sky_music/ui/picker_metadata.py:83`). Add
  `min_hold_margin_us` to each such list.
- **Validation mirror (mandatory):** `domain/validation.py:_frame_coupled_us` (lines 191-205) is
  documented as the identical computation (`scheduler_types.py:190-194` comment). Add the same
  margin there, in the same branch, reading the same profile key with the same default — the
  `min_hold_us <= frame_us` unsafe check (`validation.py:273-275`) must evaluate the
  margin-included value. Do not change `ABSOLUTE_MIN_HOLD_US` or the `>= 10_000` built-in floor
  logic.

### 3.3 Documentation updates (required, same commit)

`docs/timing-principles.md`:

- §0 currently states "**No Artificial Margins**" — that claim becomes false. Rewrite the bullet:
  same-key feasibility is still governed by `min_hold_us` with no *arbitrary latency* margin, but
  `min_hold_us` itself now includes a small constant **device-delivery margin**
  (`min_hold_margin_us`, default 500 µs) covering the residual kernel delivery latency after
  SendInput returns (previously acknowledged but unaccounted).
- §2 Hold Model formula becomes
  `hold_us = min_hold_us = ceil(min_hold_frames × frame_us) + min_hold_margin_us`.
- §3 Rationale note ("Residual completion latencies ... not accounted for"): update to "now
  covered by `min_hold_margin_us`".
- Mention the Phase 6 plan: the constant becomes per-device once the input-delivery calibration
  harness exists.

Also update the two in-code "timing honesty" comments that say the residual is "not explicitly
accounted for": `engine.py:441-444` (PlaybackEngine docstring) and `coordinator.py:296-299`
(`activate_sent_downs` docstring) — both now reference the margin.

### 3.4 Expected test churn — recompute by formula, do not regenerate

Effective values change ONLY where a frame model with fps is materialised. Arithmetic:

| Profile @ fps | Old `min_hold_us` | New (`+500`) |
|---|---|---|
| local_precise @144 (frames=1.0, frame=6945) | 6 945 | 7 445 |
| local_precise @60 (frame=16 667) | 16 667 | 17 167 |
| balanced @144 (frames=1.02 → ceil ratio: round(1.02×6945)=7084) | 7 084 | 7 584 |
| audience_safe @144 (1.5 → 10 418) | 10 418 | 10 918 |

Steps:

1. `grep -rn "6945\|6944\|7084\|16667\|10418" tests/` — every hit that asserts a materialised
   hold/min_hold must be re-derived with the table above. Known hits: `tests/make_test_song.py:96`
   (comment only — update the comment), `tests/test_phase1_metadata.py:21-27` (comments + the
   equality-boundary assertion in `test_schedule_metadata_short_note_counter`).
2. `test_phase1_metadata.py` boundary flip: at 60 fps `local_precise` min_hold becomes
   17 167 > 16 667, so the "not a short note because equal" case becomes strictly greater — the
   assertion `sub_60fps_frame_notes == 0` still holds (hold is now LONGER than one 60 fps frame).
   Verify, don't assume. While editing this file also perform the Phase 5C comment cleanup if
   Phase 5 has not run yet (avoid touching the file twice).
3. Golden schedules / telemetry goldens under `tests/golden_schedules/`: any stored
   `min_hold_us` or release timestamps shift by exactly +500 (releases move because holds
   lengthen). Verify each changed number differs from the old one by exactly the margin (or by 0
   where no frame model applies). If any delta is NOT exactly the margin, STOP — the
   implementation is wrong (most likely margin leaked into the unframed branch or into overrides).
4. Same-key feasibility thresholds rise by 500 µs: tests constructed near the old floor
   (e.g. synthetic songs with interval == old min_hold) may flip from feasible to
   infeasible/moderate. Adjust the synthetic intervals to preserve each test's intent (stated in
   its name), rather than weakening assertions.
5. Add NEW tests:
   - margin default: `FrameTimingPolicy.from_profile_name("local_precise", fps=144).min_hold_us == 7445`;
   - margin zero escape hatch reproduces 6 945;
   - explicit `min_hold_override_us` in a profile dict is honoured verbatim (no margin);
   - unframed path (fps=None) unchanged;
   - `validate_timing_profile` accepts the built-ins at 144 fps (mirror stays consistent).

**Commit:** `feat(timing): add min_hold_margin_us device-delivery margin (default 500us) to the frame model`

---

## Phase 4 — `gap_below_frame` diagnostic: same-key repeat visibility gap

> **Why.** Current feasibility (`interval ≥ min_hold`) only guarantees the FIRST note's hold is
> visible. For the game to register a re-press, at least one game frame must sample the *released*
> state between up₁ and down₂ — the gap `interval − actual_hold` needs ~≥ 1 frame; below that the
> repeat registers only with probability ≈ gap/frame. The scheduler already computes
> `min_same_key_up_gap_us` (`scheduler.py:308-311`) but never warns. Real-corpus songs are far
> above the threshold (min interval 76 ms), so this is a diagnostic, NEVER a scheduling change.

### 4.1 Change (scheduler.py, `build_key_actions`)

In the per-draft loop, right where `same_key_up_gap_us` is computed (`scheduler.py:308-311`), add:

```python
if (
    int(policy.frame_us) > 0
    and planned_hold.risk != "severe"
    and same_key_up_gap_us < int(policy.frame_us)
):
    gap_below_frame_repeats += 1
    diagnostics.append(ScheduleDiagnostic(
        source_index=draft.source_index,
        note_key=draft.note_key,
        scan_code=draft.scan_code,
        code="gap_below_frame",
        message=(
            f"Release-to-repress gap {same_key_up_gap_us / 1000:.1f}ms is below one frame "
            f"({int(policy.frame_us) / 1000:.1f}ms); the game may sample the key as continuously "
            "held and miss the repeat."
        ),
    ))
```

- Initialise `gap_below_frame_repeats = 0` alongside the other counters (`scheduler.py:212-217`).
- Exclude `risk == "severe"` (those already produce `impossible_repeat`; double-flagging is noise).
- Add ONE aggregated warning following the `sub_60fps_frame_notes` pattern
  (`scheduler.py:378-383`), emitted when the counter > 0: recommend lowering tempo or accepting
  probabilistic repeats.
- Add a short code comment noting the chord-stagger interplay: the gap is evaluated pre-stagger,
  and `apply_chord_stagger` only pushes downs LATER (gap can only grow), so the check is
  conservative — correct as-is.

### 4.2 Types / metadata

- Extend the `ScheduleDiagnostic.code` Literal (`scheduler_types.py:254`) with
  `"gap_below_frame"`.
- Add `gap_below_frame_repeats: int = 0` to `ScheduleMetadata` (`scheduler_types.py:259`) and
  populate it in the `build_key_actions` return.
- Telemetry passthrough: `grep -rn "sub_60fps_frame_notes" src/` — mirror `gap_below_frame_repeats`
  into exactly the same consumer set (schedule summary / picker metadata), no more, no less. If
  `sub_60fps_frame_notes` reaches `telemetry.schedule_summary`, register the new key as OPTIONAL in
  `telemetry_summary_schema_v1.json` (same treatment as Phase 1.3).

### 4.3 Tests

New test file or extend `tests/test_phase1_metadata.py`:

- Synthetic song, `local_precise@144` (post-Phase-3 min_hold 7 445): two same-key notes 8 ms apart
  → interval 8 000 ≥ 7 445 (feasible, not severe), hold compressed to 8 000? No — hold =
  min(target 7 445, interval 8 000) = 7 445, gap = 555 µs < frame 6 945 → counter == 1, warning
  present, diagnostic code `"gap_below_frame"`.
- Two notes 500 ms apart → counter == 0.
- A severe (interval < min_hold) case → counted ONLY as `impossible_repeat`, `gap_below_frame_repeats == 0`.

**Commit:** `feat(scheduler): diagnose same-key repeats whose release gap is below one game frame`

---

## Phase 5 — Simplification: retire onset_bias, RLS linear model, and the lead disk cache

> Approved by the maintainer. Removes working, tested machinery whose only benefit is sub-frame
> and therefore unobservable to the frame-sampling game. **KEEP (load-bearing, do not touch):**
> per-polyphony send EMA, residual/prologue EMA with the 500 µs cap, the fallback chain
> nearest-bucket→total→0, the `max_lead_us` clamp, and the no-early-conflict guard in the
> coordinator.

### 5A — Remove `onset_bias_us` (all sites verified by grep)

| File | Sites |
|---|---|
| `src/sky_music/orchestration/core/loop.py` | ctor param `:244`, assignment `:255`, `get_current_leads` addition `:304`, `_down_lead_for_batch` additions + comment `:307-315` |
| `src/sky_music/orchestration/engine.py` | ctor param `:480`, `self.onset_bias_us` `:525`, pass-through to `DispatchLoop` `:730` |
| `src/main.py` | argparse `--onset-bias-us` definition (grep for it), `RUNTIME_STATE.onset_bias_us = ...` `:488`, call sites `:840`, `:918` |
| `src/sky_music/orchestration/runtime_session.py` | fields `:16`, `:43` |
| `src/sky_music/cli/console_playback.py` | resolution `:411`, engine kwarg `:613` |

After removal, `_down_lead_for_batch` for downs becomes plain
`self.estimator.get_lead_us(ActionKind.DOWN, len(batch.intents))` (or `dispatch_lead_us` when
set), and `get_current_leads` returns the estimator/scalar values without addition. Check
`get_current_leads` consumers (`grep -rn "get_current_leads"`) — the HUD/tests read it; its
signature stays `tuple[int, int]`.

CLI note: removing `--onset-bias-us` is an explicitly approved CLI change. Remove the flag
entirely (no deprecated no-op).

Tests: `tests/test_adaptive_lead.py:630-655` (`test_down_lead_for_batch_onset_bias_is_onset_only`
and the `_bias_engine` helper's `onset_bias_us` parameter) and
`tests/test_runtime_dispatch.py:1185-1217` (`test_onset_bias_us_is_applied_only_to_downs`).
Delete the two bias-specific tests; keep `_bias_engine` (rename if now misnamed) for the other
tests that use it — adjust its signature, do not delete shared helpers.

### 5B — Retire the RLS linear model + cross-session disk cache

All in `src/sky_music/orchestration/engine.py` unless noted:

1. **`SendLatencyEstimator`:**
   - Delete `_predict_linear` (`:265-280`) and the six `_lin_*` fields (slots `:139-144`, init
     `:172-177`) plus the RLS accumulator block in `update()` (`:221-228`).
   - In `update()`, delete the warm-start branch (`warm_base = self._predict_linear(n)` …,
     `:195-201`); keep ONLY the classic accumulate-then-seed path for cold buckets.
   - In `get_lead_us()`, delete the `predicted = self._predict_linear(n)` branch (`:290-292`).
     Resulting down chain: exact warm bucket → nearest warm bucket ≤ n → total EMA (after
     `_SEED_SAMPLES`) → 0.
   - Delete `export_state` / `import_state` (`:308-411`).
   - Rewrite the class docstring (`:84-119`): remove linear-model/warm-start/cache paragraphs;
     KEEP the residual-cap honesty note verbatim.
   - Constructor param `lin_forget` and its docstring sentence go away.
2. **Module level:** delete `_LEAD_CACHE_PATH` (`:414`), `load_lead_cache` (`:417-424`),
   `save_lead_cache` (`:427-436`).
3. **`PlaybackEngine`:** delete ctor param `lead_cache_path` (`:482`), the import-at-init block
   (`:517-522`), the `_lead_cache_enabled` property (`:608-616`), and the save-in-finally block
   (`:909-912` — remove the whole `if self._lead_cache_enabled:` stanza including its comment).
4. **Callers:** `console_playback.py:392` (remove `_LEAD_CACHE_PATH` from the import) and `:614`
   (kwarg); `ui/textual_app/app.py:865` and `:940` (same).
5. `.cache/lead_estimator.json` on disk: leave it; it is gitignored and now simply unread. Do not
   add deletion code.

Tests (`tests/test_adaptive_lead.py`): the warm-start-from-linear, extrapolation, and cache
round-trip tests (`:747-758` region and any `import_state`/`export_state`/`_predict_linear`
references — grep the file) must be **repointed, not deleted**: assert the surviving fallback
chain instead — (a) unseeded bucket with a warm smaller bucket uses nearest ≤ n; (b) no warm
bucket but warm total uses total; (c) fully cold returns 0; (d) bucket seeds after
`_SEED_SAMPLES` samples. Cold-start behaviour change to document in the commit message: the first
few notes of each chord size per session now dispatch with lead 0 (a few tens of µs late —
sub-frame, accepted by the maintainer).

### 5C — Test hygiene

`tests/test_phase1_metadata.py:21-27`: replace the stray drafting comments ("Wait, the check in
scheduler is..." / "It will trigger compression/drops... ? No, the second note overlaps.") with a
clean, factual comment stating the arithmetic (fold into the Phase 3 edit of this file if both
phases land together).

**Commits:**
- `refactor(dispatch): remove onset_bias_us knob (approved; game cannot observe sub-frame bias)`
- `refactor(engine): retire RLS linear model and lead disk cache; keep per-poly EMA + residual`

---

## Phase 6 — SEPARATE TRACK (own PR, human-gated): per-device input-delivery calibration

> **Goal.** Replace the constant 500 µs margin with a measured, per-device value. This phase
> requires running on real hardware and a human smoke test; an AI agent may implement it but the
> resulting numbers must be reviewed before changing any default.

### 6.1 What is measured

SendInput-return → OS delivery latency, separately for key-down and key-up, on the app's OWN
window. The hold-shrink risk is the asymmetry `down_delivery − up_delivery`; the margin must cover
its high percentile.

### 6.2 Design constraints (P0-driven, non-negotiable)

- `SendInput` only. The capture target is a window OWNED BY THIS APP. No game interaction of any
  kind.
- **Refuse to run if Sky is running at all** (`inputs.get_sky_window() is not None` → abort with a
  clear message). Injected keys land in the foreground window; the calibration window must be
  foreground (verify via `GetForegroundWindow()` == own hwnd before EVERY injection; abort on
  mismatch).
- Raw input registration: `RegisterRawInputDevices` with `RIDEV_INPUTSINK` on the app's window,
  usage page/usage = generic desktop / keyboard. Timestamp `time.perf_counter_ns()` at SendInput
  return and in the `WM_INPUT` handler; the message pump runs on a dedicated thread with the same
  clock.
- N ≈ 200 down/up pairs of ONE Sky scan code, ≥ 20 ms apart (no autorepeat interference), keys
  swallowed by the calibration window.
- Output: `.cache/input_latency.json` —
  `{"version": 1, "down_us": {"p50":…, "p90":…, "p99":…}, "up_us": {…}, "sampled_at": iso8601, "n": 200}`.
  Validate on read exactly like the retired `import_state` did: type/range-check every field,
  ignore a corrupt cache entirely.

### 6.3 Margin resolution order (config layer)

`min_hold_margin_us` resolution becomes: explicit profile key > measured cache recommendation >
default 500. Recommended formula (subject to human review after first real captures — put this
sentence in the code comment):

```
margin_rec = clamp(300, 2000, p99(down_delivery) − p50(up_delivery) + 100)
```

### 6.4 Placement

New CLI entry: extend `src/sky_music/cli/doctor_command.py` (or `calibration_command.py` — read
both first and follow the existing subcommand pattern) with `calibrate-input-latency`. Win32
plumbing lives in `src/sky_music/platform/win32/` behind the existing module boundary — nothing in
`orchestration.core` may import it (boundary test `tests/test_core_boundary.py` enforces this).

### 6.5 Tests

Unit-test the cache read/validation and the margin formula with synthetic distributions. The
win32 pump itself is smoke-tested manually (`uv run python -m app doctor calibrate-input-latency`)
— state in the PR that unit tests cannot prove timing.

---

## 7. Explicit non-goals (do NOT do these anywhere in this plan)

- Do NOT lower or remove the 700 µs spin floor; do NOT add `_mm_pause`/yield to the spin.
- Do NOT clamp/override/second-guess the user's configured fps.
- Do NOT reintroduce any late retry for note-ons (same-frame retry stays exactly as shipped).
- Do NOT change the completion-anchor contract (`release_not_before = down_completion + min_hold`).
- Do NOT remove the per-polyphony EMA, the residual EMA, or the no-early-conflict guard.
- Do NOT touch chord-stagger (off-by-default remote knob) or the focus-gate logic.
- Do NOT add margin to the unframed (`fps` unknown) path or on top of explicit `*_override_us`.

## 8. Verification gates

| After | Command |
|---|---|
| Phase 1 | `uv run pytest -k "wait or strategy or spin or dispatch or telemetry"` |
| Phase 2 | `uv run pytest -k "epoch or threaded"` |
| Phase 3 | `uv run pytest -k "schedule or timing or profile or validation or metadata or golden"` |
| Phase 4 | `uv run pytest -k "schedule or metadata or diagnostic"` |
| Phase 5 | `uv run pytest -k "adaptive_lead or runtime_dispatch or cli or golden"` |
| Sequence end | `uv run ruff check . && uv run pyright && uv run pytest` |
| Phase 6 (separate PR) | full triad + manual hardware smoke test |

The P0 AST/CI audit (`scripts/audit_*`) must stay green throughout — nothing here touches the
SendInput-only mechanism (Phase 6 adds injection but only into the app's own window, still via
SendInput).

## 9. Risk register

| Risk | Likelihood | Mitigation |
|---|---|---|
| Phase 3 margin leaks into unframed path or overrides | Medium (subtlest phase) | §3.1 rules 3–5; golden deltas must equal exactly +margin; zero-margin escape-hatch test |
| Phase 3 shifts a near-floor synthetic test's meaning silently | Medium | §3.4 step 4: adjust inputs to preserve test intent, never weaken assertions |
| Phase 5B removes a fallback some test depended on implicitly | Low | Repoint, don't delete; full-suite gate; the surviving chain already existed |
| Phase 2 breaks a hidden default-False dependency | Low | Only one test asserts the default (listed); rebase never fires in direct mode |
| Phase 1 cap change hides a command-latency regression | Very low | Loop polls at 2 ms regardless; event mode (production default) unaffected |
| Phase 6 injects keys into the wrong window | Low, high impact | Foreground re-check before every injection; refuse to run while Sky exists; own-window sink |

## 10. Commit sequence (surgical, reviewable)

1. `perf(wait): align polled sleep cap with 2ms command poll; flag polled degradation in telemetry`
2. `fix(engine): align epoch_rebase constructor defaults with production-on runtime state`
3. `feat(timing): add min_hold_margin_us device-delivery margin (default 500us) to the frame model`
4. `feat(scheduler): diagnose same-key repeats whose release gap is below one game frame`
5. `refactor(dispatch): remove onset_bias_us knob (approved; game cannot observe sub-frame bias)`
6. `refactor(engine): retire RLS linear model and lead disk cache; keep per-poly EMA + residual`
7. *(separate PR, human-gated)* `feat(doctor): per-device SendInput delivery-latency calibration + measured min_hold margin`
