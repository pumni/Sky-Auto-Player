# Plan: Busy-spin & adaptive-lead audit + hot-path / accuracy refinements

> **Status:** Proposed (2026-07-18). Author: deep-review round 3 (critical pass).
> **For:** an AI refactor agent. Follow `AGENTS.md` exactly. This plan overrides prior
> intuition but NOT the P0 security mandates.
> **Companion to:** `docs/2026-07-18_retry-reenable-and-jitter-refinement-plan.md`
> (that one shipped: same-frame retry + Phase 2/3 allocations/telemetry). This plan does
> NOT repeat those; it audits the two mechanisms the user asked about (busy-spin,
> adaptive-lead) and lists the remaining, defensible optimizations.

---

## 0. Context, thesis, and scope decision (read first)

The dispatch → `SendInput` sender is at the `perf_counter` noise floor: real telemetry has
visible-lateness p99 < 1 ms, 0 notes > 5 ms late — **~18–700× tighter than one game frame**
(6.9 ms @144 fps, 16.7 ms @60 fps). The precision ceiling is the game's **frame-quantized input
sampling**, which the sender cannot phase-lock to without reading game state (**forbidden by P0**).

**Therefore the governing rule of this plan:** *any change whose only effect is sub-frame sender
precision is unobservable to the game and is NOT worth doing.* Both mechanisms under audit
(busy-spin, adaptive-lead) already deliver sub-frame precision; their **defensible value is
tail-taming** (keeping the worst-case send from crossing a frame boundary), not p50 precision. The
audits below judge them on *that* basis, not on "how tight can we make p50."

### Scope decisions (hard boundaries)

- **FPS stays the user's choice.** `min_hold` derives from configured `fps`
  (`scheduler_types.py:188-207`). Do **not** clamp/override/floor/second-guess fps anywhere.
  Phase 1 only makes the *assumption visible*; it never changes it.
- **No sender-precision chase.** Do not add `_mm_pause`/yield to the spin, do not tighten the
  spin below the tail-safety guard, do not add more lead sophistication for p50. (Rationale in
  Audit A/B.)
- **P0 immutable:** `SendInput` only; no game memory/hooks/injection; validate inputs strictly.

---

## Audit A — Busy-spin: is it best practice, and is the logic correct?

**Where:** `wait_strategy.py:41-56` (`spin_until_us`), `:57-121` (`wait_until_us`);
guard sizing `engine.py:721-752` (`_probe_timer_wake_error`); threshold read `engine.py:612-615`.

### A.1 Verdict — best practice: YES (with one honest caveat)

The mechanism is the modern Windows high-precision-scheduling recipe, and it is implemented
correctly:

- **High-resolution waitable timer** (`CREATE_WAITABLE_TIMER_HIGH_RESOLUTION`) to sleep to
  `target − guard`, then a **short busy-spin** to the exact deadline. This is strictly better than
  the classic `timeBeginPeriod(1)+Sleep+spin`: the high-res timer is period-independent (measured
  p99 wake ≈ 0.57 ms with or without `timeBeginPeriod`), so the process-wide 1 ms period was
  correctly retired and the timer guard is now fallback-only (`playback_supervisor.py:317-341`).
- **TIME_CRITICAL / MMCSS priority** on the dispatch thread (`rt_priority.py`) + **EcoQoS
  power-throttling opt-out** (`inputs.disable_thread_power_throttling`) so the OS does not stretch
  the spin. Correct: a spin under a throttled core is the classic silent-jitter trap.
- **Pure spin (no yield) for the final guard.** On a TIME_CRITICAL thread that intends to own the
  deadline, `SwitchToThread`/`Sleep(0)` risk quantum-scale delays — the prior review correctly
  retracted adding them. Under free-threaded 3.14t the spin thread runs on its own core in true
  parallel with the UI thread, so the pure spin does not starve Python bytecode elsewhere.

**Why `_mm_pause` is (still) rejected — with the honest reason, not a hand-wave:** the spin's p50
tightness is unobservable to the game, so `_mm_pause` would cost *nothing observable* if it were
free. But in **pure Python** the only way to emit `PAUSE` per iteration is a `ctypes` call whose
Python→C overhead (~50–100 ns) *exceeds* the pause latency it inserts and *replaces* the natural
~20–30 ns spacing of `perf_counter_ns()`. Net: more overhead, coarser deadline detection, no
observable benefit. Rejected — but the accurate reason is "Python call overhead > benefit," not
"precision matters." (A C-extension spin could revisit this; out of scope — the CPU saving is
sub-1% of one core and the game cannot see it.)

### A.2 Verdict — logic correct: YES

- `spin_until_us` compares `perf_counter_ns()` against `target_system_us * 1000`; the µs→ns
  round-trip loses ≤ 999 ns of the target — utterly below a game frame. Correct.
- The `remaining <= spin_threshold` branch spins the whole remainder (dense passages), else sleeps
  to `target − guard` then spins `guard` (sparse). Idle inter-note gaps therefore cost **0 % CPU**
  (timer sleeps the whole gap); the spin only runs the final `guard` per note. Confirmed correct.
- Event mode wakes on `WaitForMultipleObjects(timer, command_event)` so a pause/panic during the
  pre-note sleep interrupts promptly; only the final `guard` (< 1 ms) is non-interruptible — human-
  imperceptible. Correct.

### A.3 The one caveat worth acting on: guard *sizing*, not the spin itself

`_probe_timer_wake_error` sizes the guard from **10 samples** of a 2 ms sleep as
`max(700, min(3_000, mean + 3σ + 100))` (`engine.py:730-741`).

- **10 samples → noisy σ**; a transient hiccup during the probe inflates the guard for the whole
  song (mean+3σ is sensitive to one outlier at n=10).
- **The 700 µs floor** forces ≥ 700 µs of spin/note even on a clean machine whose timer wakes
  within ~300 µs. Cost: `(700 − actual) × note_rate` ≈ **≤ 0.8 % of one core** — small, and it is
  *buying tail safety*, so lowering it trades a real safety margin for a marginal CPU win.

**Decision:** improve the *estimator quality* (pure upside), do **not** lower the floor by default.
See Phase 3.

**Overall Audit A verdict: the busy-spin is correct and best-practice-aligned. The mechanism is
not a candidate for removal or rework. Only the guard-sizing probe is refinable.**

---

## Audit B — Adaptive-lead: is it best practice, and is the logic correct?

**Where:** `engine.py:84-407` (`SendLatencyEstimator`); applied at `loop.py:294-315`
(`_down_lead_for_batch` / `get_current_leads`); feedback at `loop.py:627-636`; deadline at
`coordinator.py:175-213`.

### B.1 Verdict — logic correct: YES (no bug found after line-by-line review)

Reviewed for real defects; the model is internally consistent:

- **Feed-forward + feedback split is sound.** The per-polyphony EMA (`_ema_down[n]`) predicts the
  `SendInput` syscall duration; the residual EMA (`update_completion_error`, fed
  `visible_lateness_us`) catches the *prologue* (spin overshoot + Python work between spin-end and
  `SendInput`) that the send-EMA cannot see. Two distinct error sources, two estimators. Correct.
- **Over-lead cannot cause dropped notes.** Lead is clamped to `max_lead_us` (2 ms) and the
  coordinator's `_early_pop_blocked` guard (`coordinator.py:160-173,277-281`) defers an early pop
  while the scan code is still active/pending, so a too-large lead degrades to "dispatch at authored
  time," never to `dropped_conflict`. Correct and important.
- **The residual asymmetry is deliberate AND correct for this domain.** `_residual_bias_us` returns
  `max(0, …)` — it pulls lead *earlier* on late completions but never *later* on early ones. For a
  frame-quantized target, landing slightly early is strictly safer than late (early lands in the
  same or a prior frame; late risks the next frame). So the "never shrink lead on early completion"
  rule is the right call, not a bug. The send-EMA still trends down as real (shorter) sends arrive,
  so lead does not run away upward.
- **Warm-start / linear extrapolation / cross-session cache** are range-checked on import
  (`import_state`) and clamped again on read (`get_lead_us`), so a corrupt cache cannot inject an
  absurd lead. Correct defensive posture.

### B.2 Verdict — best practice: OVER-ENGINEERED FOR THE OBSERVABLE BENEFIT (not wrong, but not "best")

This is the honest, non-flattering assessment the user asked for:

- The estimator's **output magnitude is ~50–200 µs** of lead. That is **sub-frame → unobservable to
  the game.** Its *only* defensible value is tail-taming: pre-leading a chord that *occasionally*
  takes ~1–2 ms to send so even the slow send completes near its frame instead of the next one.
- For that tail goal, the load-bearing part is **per-polyphony bucketing of the send-EMA** (a 5-key
  chord's worst-case send is genuinely longer than a 1-key note's). The **RLS linear model with
  exponential forgetting + warm-start + the on-disk cross-session cache** are a lot of moving parts
  (~200 lines, disk I/O, a `.cache/lead_estimator.json`) for warm-starting rare chord sizes — an
  effect the game cannot perceive on the first few occurrences anyway.
- **Conclusion:** logic is correct; best-practice-wise it is *more* machinery than the observable
  benefit justifies. It is a **simplification candidate, not a fix.** See Phase 4 (optional, gated).

### B.3 One honesty fix (cheap)

The design comments imply completions "land on schedule." With the residual cap at
`_MAX_RESIDUAL_US = 500` (`engine.py:119`), a machine whose prologue exceeds 500 µs will land
**persistently up to ~500 µs late** — fine for the frame-quantized game, but the wording overstates.
Phase 4.1 adds one sentence so the claim is "within ~½ ms of schedule," matching reality.

**Overall Audit B verdict: keep it (correct, cheap once warm), but it is the codebase's clearest
example of complexity exceeding observable payoff. Optional simplification in Phase 4.**

---

## Phase 1 — Make the fps↔min_hold assumption VISIBLE (the one real accuracy hole)

> This is the highest-value item in the plan. It is the only place where the game can genuinely
> **fail to receive a note per its timestamp**, and it is invisible to sender-side telemetry.

**Problem.** `min_hold` = `min_hold_frames × ceil(1e6 / configured_fps)`
(`scheduler_types.py:194-202`). If the user configures `fps` **higher** than the game's *real*
frame rate, `min_hold` is sized to a frame *shorter* than a real frame. A short note's down+up can
then fall inside a single real game frame → **the game never registers the press**, while telemetry
stays `sender_clean`. Existing `validate_timing_profile` (`validation.py:254-280`) only checks
`min_hold_us > frame_us` **at the configured fps** — it cannot catch a configured-vs-real mismatch
because the tool may not read the real fps (P0).

**What we CAN do (and must not over-promise): make the assumption legible, do not fake a detector.**

### 1.1 Surface the active assumption in telemetry runtime options (always)
`engine.py` already records `fps` and `min_hold_us` into telemetry. Add a derived, explicit line so
any summary/log states the contract in one place:
```
"min_hold_assumes_fps": <configured_fps>,
"min_hold_us": <value>,
"note": "min_hold is sized for the CONFIGURED fps; if the game runs slower, short notes may
         land within one real frame and not register."
```
Purely additive to `record_runtime_options`; zero runtime cost; no behaviour change.

### 1.2 One advisory in `doctor` / HUD when configured fps is aggressive
`cli/doctor_command.py` (and/or the HUD startup line) should emit **one** advisory when
`configured_fps > 60` **and** any scheduled note's authored down→up hold `< ceil(1e6/60)` (i.e. the
song contains notes that would be sub-one-60fps-frame). Message (advisory, not error):
> "This profile assumes {fps} fps. {k} short note(s) are shorter than one 60 fps frame; if your
> game runs below {fps} fps they may not register. Lower fps in the profile or use `local_precise`."

The `frame_lateness` diagnostic code already exists in the vocabulary
(`scheduler_types.py:254`) — reuse it; do not invent a new one. Count `k` during schedule build
(the `same_key`/hold planning loop already computes `actual_hold`), so this is a cheap counter, not
a second pass.

### 1.3 Docs
Add a short "fps vs real game fps" section to `docs/timing-principles.md` (or the profile docs)
stating the failure mode and that the tool honours the configured fps by design.

### 1.4 Tests
- `test_schedule_metadata`: a song with notes shorter than one 60 fps frame under a 144 fps profile
  sets the short-note counter > 0; a `local_precise`/60 fps profile sets it 0.
- `test_telemetry_summary_schema`: `min_hold_assumes_fps` present and equals configured fps.
- Advisory text asserted in a doctor test with a synthetic short-note song.

---

## Phase 2 — Hot-path allocation trims (Finding A concrete; Finding B optional)

> Honest magnitude: sub-microsecond per note, but real allocations on a GC-paused hot path (each
> lives until end-of-song), and they contradict the codebase's own hoisting principle. Finding A is
> the sibling that the prior plan's Phase 2 *missed* while fixing `_resolve_down_outcome`.

### 2.1 Finding A — drop three redundant `tuple(genexpr)` copies of already-tuples
`ExecutionResult.sent_scan_codes` / `skipped_scan_codes` are **already `tuple[int, ...]`**
(they are `InputSendResult.sent` / `.skipped_duplicates`, `backend.py:38-44`). These rebuild an
identical tuple per note-batch:

- `loop.py:639` → `activate_sent_downs(playable, tuple(scan_code for scan_code in result.sent_scan_codes), …)`
- `loop.py:687-688` → `complete_releases(releases, tuple(sc for sc in result.sent_scan_codes), tuple(sc for sc in result.skipped_scan_codes))`
- `loop.py:745-746` → same two in the multi-release path

**Fix:** pass `result.sent_scan_codes` and `result.skipped_scan_codes` **directly**.
`activate_sent_downs` (`coordinator.py:287-344`) and `complete_releases` (`coordinator.py:425-459`)
only *read* the tuples (membership / `set()` / index) and never mutate them, and tuples are
immutable — so sharing the reference is safe. Byte-identical behaviour; the golden-timeline test
must stay green.

### 2.2 Finding B (optional, low value) — `_down_lead_for_batch` computed twice per down
`_drain_due` (`loop.py:1102-1111`) passes `lead_for_batch=self._down_lead_for_batch` to
`pop_due_authored` (which calls it to test dueness, `coordinator.py:273`), then recomputes
`down_lead = self._down_lead_for_batch(batch)` at `loop.py:1110`. No `estimator.update` runs
between the two calls, so the value is identical — one redundant estimator read per down.

**Magnitude is genuinely tiny** (a warm-bucket `get_lead_us` is ~10 ops). Do **only** if it stays
clean: the least-invasive form is to have `_dispatch_down_batch` accept an *optional precomputed*
`lead_down` and let `_drain_due` reuse the value it already has — **but** `pop_due_authored` does not
return the per-batch lead, so a clean single-source requires either (a) `pop_due_authored` yielding
`(batch, lead)` tuples, or (b) accepting the recompute. **Recommendation: SKIP unless doing a
broader drain refactor.** It is listed for completeness and to record that it is *known and
deliberately deferred*, not overlooked. Do not contort the code for ~10 ops/note.

### 2.3 Tests / gate
`uv run pytest -k "golden or dispatch or loop or coordinator"` then full suite. Finding A cannot
change any observable output; if a golden timeline moves, the change was wrong.

---

## Phase 3 — Spin guard probe robustness (pure upside; keep the floor)

> Acts on Audit A.3. Improves the *estimate*, not the mechanism. Do **not** lower the default floor.

### 3.1 Make the wake-error probe less noisy
In `_probe_timer_wake_error` (`engine.py:721-752`):
- Raise the sample count from **10 → 30** (still < 100 ms of probe time; runs before the perf
  anchor, so it does not compress first onsets).
- Replace `mean + 3σ` with a **robust high percentile of the samples** (e.g. `p90` of the 30 wake
  errors, `+ 100 µs` margin), which is less outlier-sensitive at small n than mean+3σ and better
  represents "the guard I actually need to beat the timer's realistic overshoot."
- Keep the `max(700, min(3_000, …))` clamp **as-is by default.**

### 3.2 Make the floor configurable for advanced low-power users (opt-in only)
Expose the `700` floor as a config/CLI knob (e.g. `--spin-floor-us`, default **700**). Rationale:
the floor is a deliberate ≤ 0.8 %-of-a-core safety margin; a user targeting a low-power/handheld
device may accept slightly higher tail-lateness risk to reclaim it. **Default unchanged**; document
that lowering it trades tail safety for CPU and should be validated with `tests/bench_sendinput.py`.

### 3.3 Gate
`uv run pytest -k "adaptive_spin or probe"`. The probe change is deterministic given a fake sleeper;
assert the new percentile logic and the unchanged default floor. Real-hardware effect must be spot-
checked with the bench script (unit tests cannot prove timing) — note this in the PR, do not claim
a CPU win from unit tests alone.

---

## Phase 4 — (OPTIONAL, GATED) adaptive-lead simplification + honesty note

> Acts on Audit B. **Do not implement the simplification without explicit human approval** — it
> removes working, tested machinery. The honesty note (4.1) is cheap and safe; do that regardless.

### 4.1 Honesty note (safe, do it)
Update the lead/hold design comment (`coordinator.py:332-341` region and/or
`engine.py:84-119` docstring) to state the completion lands **within ~`_MAX_RESIDUAL_US` (≈ ½ ms) of
schedule**, not exactly on schedule. One or two sentences; no code change.

### 4.2 Simplification proposal (approval-gated, separate PR)
If maintenance-surface reduction is wanted: **keep** per-polyphony send-EMA + residual bias (the
tail-taming core); **consider retiring** the RLS linear model + on-disk cross-session cache
(`_predict_linear`, `export_state`/`import_state`, `save_lead_cache`/`load_lead_cache`,
`.cache/lead_estimator.json`). Justification: they warm-start rare chord sizes across sessions, an
effect the frame-quantized game cannot perceive on the first few occurrences. Replacement for an
unseeded bucket: nearest seeded bucket ≤ n → total-down EMA → 0 (the fallbacks already exist in
`get_lead_us`). **Risk:** the first occurrence of a rare large chord in a cold session leads with a
smaller bucket's lead until it seeds (~5 samples) — a sub-frame difference. **Only do this if the
team decides the ~200 lines + disk I/O are not worth it;** it is explicitly optional and reversible.

### 4.3 Tests (only if 4.2 is approved)
`test_adaptive_lead` currently asserts warm-start-from-linear and cache round-trip; those tests
would be updated to the fallback-chain behaviour. Do not delete coverage — repoint it.

---

## Phase 5 — (SEPARATE TRACK) onset_bias calibration via audio loopback

> This is the **only** remaining lever that changes what the game *perceives* (phase-centering the
> frame-quantization error), and it is a **measurement** task, not a code-precision task. Scope it as
> its own effort; it does not block Phases 1–4.

**Idea.** Because the game quantizes to frame boundaries, every note is on average ~½ frame late as
perceived. A static `onset_bias_us` ≈ ½ frame (`loop.py:304-315` already applies it, onset-only)
would **center** the perceived-onset distribution on the authored time instead of biasing it late.
But it must be **measured**, not guessed, because the game's real fps is unknown and dynamic.

**Blocking gap:** the WASAPI loopback harness exists (`tests/audio_loopback.py`,
`tests/measure_stutter_live.py`) but the committed `.wav` captures are ~0.18 s aborted files —
**there is no usable SendInput→audible-onset measurement yet.**

**Deliverables (measurement first, code second):**
1. Fix/complete the live-capture path so `measure_stutter_live.py` produces a full-length capture
   correlated to the telemetry CSV (`dispatch_completed_us` ↔ detected audio onset).
2. From N real captures, estimate the SendInput→audible offset distribution and its dependence on
   the game's observed frame period.
3. **Only then** recommend a default `onset_bias_us` (or document that it must stay per-user because
   it depends on the game's live fps). Do not ship a guessed bias.

**Do not** implement any bias-related default from theory alone — that would be exactly the
unvalidated sender tweak this plan forbids.

---

## Verification gates (per AGENTS.md altitude table)

| After | Command |
|---|---|
| Phase 1 (visibility) | `uv run pytest -k "schedule or telemetry or doctor or metadata"` |
| Phase 2 (loop.py trims) | `uv run pytest -k "golden or dispatch or loop or coordinator"` |
| Phase 3 (probe) | `uv run pytest -k "adaptive_spin or probe"` + `tests/bench_sendinput.py` spot-check |
| Phase 4.1 (comment only) | `uv run ruff check .` |
| Phase 4.2 (if approved) | `uv run pytest -k "adaptive_lead"` |
| Any code change | `uv run ruff check . && uv run pyright && uv run pytest` |

The AST/CI P0 audit (`scripts/audit_*`) must stay green — nothing here touches the `SendInput`-only
mechanism, so it should not trip, but confirm.

---

## Explicit non-goals (do not do these)

- **Do not** clamp/override/floor the user's configured `fps` (Phase 1 makes it visible only).
- **Do not** add `_mm_pause`/yield to the spin, or lower the default spin floor (Audit A / Phase 3).
- **Do not** add more lead sophistication for p50 precision (Audit B) — the game cannot see it.
- **Do not** implement Phase 4.2 (simplification) or any `onset_bias` default (Phase 5) without,
  respectively, human approval and a real loopback measurement.
- **Do not** refactor unrelated code or contort the drain loop for Finding B's ~10 ops/note.

---

## Risk register

| Risk | Likelihood | Mitigation |
|---|---|---|
| Finding A alias breaks a hidden mutator of the tuples | Very low | Coordinator only reads the tuples; tuples are immutable; golden-timeline test pins output |
| Phase 1 advisory is noisy / cries wolf | Low | Single advisory, gated on both `fps>60` AND presence of sub-60fps-frame notes; advisory not error |
| Phase 3 percentile under-guards a jittery machine | Low | Keep the 700 µs floor by default; bench-script spot-check; opt-in only for lowering |
| Phase 4.2 removes warm-start users relied on | Medium (why it is gated) | Approval-gated, reversible; fallbacks already exist; sub-frame effect |
| Phase 5 ships a guessed bias | N/A | Forbidden by non-goals until measured |

---

## Suggested commit sequence (surgical, reviewable)

1. `feat(telemetry): record min_hold_assumes_fps + short-note advisory (fps↔min_hold visibility)`
2. `perf(dispatch): drop 3 redundant sent/skipped tuple copies on the down/release hot path`
3. `perf(engine): robuster spin-guard probe (30 samples, p90+margin); expose --spin-floor-us`
4. `docs: lead completion lands within ~0.5ms of schedule (residual cap honesty)`
5. *(optional, approval-gated, separate PR)* `refactor(engine): retire RLS linear model + lead disk cache`
6. *(separate track)* `test(audio): usable live loopback capture for onset_bias calibration`
