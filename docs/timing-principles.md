# Timing Principles for Sky Music Player

This document is the engineering source of truth for designing, reviewing, and calibrating timing profiles for Sky Music Player. It defines sender-side timing contracts and profile guidance; game registration remains subject to uninstrumented frame-sampling evidence, while online audibility has additional listener-side constraints.

---

## 0. Hierarchy of Truth & Ground Truth

### Ground Truth
* **Frame-Bound Sampling:** The game samples input state once per render frame. For a key-down event to be registered, the key must remain held down for **at least 1 game frame**. This is the only hard timing constraint.
* **No Arbitrary Margins:** Same-key feasibility is determined strictly by the key's minimum hold duration (`min_hold_us`). The scheduler does not add a separate scheduling-time latency guess (such as the legacy `release_latency_margin_us`). Note that since 2026-07 `min_hold_us` itself *includes* a small constant **device-delivery margin** (`min_hold_margin_us`, default 500 µs — see §2): that margin models a measured physical effect (post-`SendInput` kernel delivery latency), not a scheduling fudge factor, and setting it to 0 restores the pure frame-ratio model.

### Evidence Hierarchy
When resolving conflicts, the following hierarchy applies:
1. **Observed Game Behavior** (audio/onsets recorded in-game) — wins over everything.
2. **Deterministic Measurements** (telemetry CSV, coordinator/scheduler simulator) — wins over "experience/intuition".
3. **Current Codebase** (`src/`) — wins over descriptions in any document.
4. **Documentation** — only interpretive; if a document conflicts with 1, 2, or 3, it is outdated/incorrect and must be corrected.

> [!NOTE]
> [AGENTS.md](../AGENTS.md) remains the single source of truth for overall project rules and coding constraints.

---

## 1. Core Terms
All timing values are expressed in microseconds ($\mu\text{s}$).

| Term | Meaning |
| :--- | :--- |
| `hold_us` | Effective key-down duration for a normal note. Derived directly from `min_hold_us` for built-in profiles. |
| `min_hold_us` | The visibility floor. The absolute minimum key-down duration allowed after compression. |
| `min_hold_frames` | The frame ratio used to calculate `min_hold_us` when FPS is known (e.g. 1.0 for `local_precise`). |
| `same_key_interval_us` | Time between two down events on the same scan code. If below `min_hold_us`, the repeat is infeasible. |
| `frame_us` | Duration of one game frame. Calculated as `ceil(1,000,000 / game_fps)`. |
| `game_fps` | The target game FPS selected or calibrated by the user. If 0 or `None`, frame-aware scaling is disabled. |
| `tempo_scale` | Playback speed multiplier. Values above 1.0 increase scheduling pressure. |

---

## 2. Timing and Feasibility Model

### Hold Model
When FPS is known and positive, built-in holds materialize from their frame ratio plus a constant device-delivery margin:
$$\text{hold\_us} = \text{min\_hold\_us} = \lceil \text{min\_hold\_frames} \times \text{frame\_us} \rceil + \text{min\_hold\_margin\_us}$$
Where:
$$\text{frame\_us} = \lceil 1,000,000 / \text{game\_fps} \rceil$$

`min_hold_margin_us` (profile key, default **500 µs**, `0` restores the pure ratio model) covers the residual kernel delivery latency after `SendInput` returns (generally <0.5 ms) and any down-vs-up delivery asymmetry — the only sender-side mechanism that can *shorten* the game-observed hold. It is applied only in the frame-model branch; explicit `hold_us`/`min_hold_us` overrides win verbatim, and the `*_unframed_us` fallback (used when FPS is unknown or disabled) gets no margin because those values already carry ample slack. The margin is a per-device allowance (planned to become measured via input-delivery calibration), not a return of the retired arbitrary `release_latency_margin_us`.

The optional Windows calibration cache is evidence of kind
`injected_raw_input_delivery_proxy`: the app-owned calibration window receives keys injected
through `SendInput`, and the harness correlates its `WM_INPUT` receipt with the native-call
completion timestamp. This is a host delivery proxy only; it does not observe Sky polling, render
frames, or audio onset. Its UTC `sampled_at` and evidence label are diagnostic metadata, and the
loader applies no freshness TTL until repeated measurements establish a conservative policy.

### FPS Assumption vs Real Game FPS
The profile's configured `game_fps` determines the length of `min_hold_us` and `hold_us`. By design, the tool strictly honors this configured FPS. If you configure a profile with a high FPS (e.g., 144) but your game is actually running at a lower FPS (e.g., 60), the generated holds will be shorter than one real frame. These "short notes" may land entirely within a single game frame and fail to register. The scheduler does not try to detect your real game FPS; it assumes the profile config is correct. If you experience dropped notes, lower the FPS in the profile or use `local_precise` at 60 FPS.

### Same-Key Feasibility
The authored feasibility floor remains:
$$\text{same\_key\_interval\_us} \ge \text{min\_hold\_us}$$

The native sender applies a stricter runtime condition because the release floor is anchored at
the actual down completion:
$$\text{repeat\_interval} \ge \text{min\_hold\_us} + \text{delivery\_budget} + \text{off\_gap\_budget}$$

`delivery_budget` is a conservative sender-side estimate, not a claim about when the game
observes the key. `off_gap_budget` is profile- and frame-dependent and is intentionally not a
universal constant. A schedule can therefore be authored-feasible while still producing a
runtime conflict under a slow or contended input path; fidelity-oriented tooling should warn or
reject such repeats instead of silently treating them as guaranteed.

If the authored interval is smaller than `min_hold_us`:
1. **Strict Mode:** The scheduler rejects the playback and recommends a lower tempo.
2. **Degraded Mode:** The scheduler preserves the minimum hold (`min_hold_us`) for the first note, which naturally overlaps the scheduled start of the second note. At runtime, the conflicting second down event is explicitly dropped to prevent stuck keys.

---

## 3. The Completion-Anchor Contract
To enforce the sender-side minimum-hold floor despite measured OS dispatch latency, key releases are scheduled relative to down-dispatch completion rather than down-dispatch start. This does not prove when the game sampled the key.

The runtime visibility contract implemented in [RuntimeDispatchCoordinator](../src/sky_music/orchestration/runtime_dispatch.py#L133) is:
$$\text{release\_not\_before\_us} = \text{down\_dispatch\_completed\_us} + \text{min\_hold\_us}$$
$$\text{effective\_release\_us} = \max(\text{scheduled\_release\_us}, \text{release\_not\_before\_us})$$

### Rationale
The sender-side completion-to-completion proxy preserves the intended hold floor: measuring from down-dispatch start would subtract the down injection latency from the sender timeline. For `local_precise` at 144 FPS (6.94 ms hold), this avoided the previously observed sender-side shortfall. Completion-anchoring does not establish a game-observed hold, because game sampling and kernel delivery are not instrumented here. The constant `min_hold_margin_us` models residual sender-to-device delivery latency; it is a margin, not game-onset evidence.

### Interaction with Adaptive Dispatch Lead (2026-06)
Since the RT-pipeline optimization, dispatch targets **onset = SendInput completion**: events are popped early by a per-kind EMA of `send_duration_us` (clamped to 2 ms) so completions land on `scheduled_us`. The lead is symmetric (downs and releases) and **the floor always wins**: a release becomes due at
$$\max(\text{scheduled\_release\_us} - \text{lead}, \text{release\_not\_before\_us})$$
and a down batch is never popped before its authored time while its key is still active or pending release (no-early-conflict guard — an early pop would otherwise become a dropped note). Live A/B on `blue` @144 FPS moved the median down-onset error from +420 µs to −3 µs with zero drops. See [rt-dispatch-architecture.md](rt-dispatch-architecture.md).

---

## 4. Profile Classes

The project defines three built-in profiles in [config.py](../src/sky_music/config.py)
(`DEFAULT_TIMING_PROFILES`, mirrored here 2026-07 — see [the source](../src/sky_music/config.py)
for the authoritative values):

* **`local_precise`:** Optimized for sharp local playback. Uses `min_hold_frames = 1.0`. Like every built-in frame-model profile it also receives the constant 500 µs `min_hold_margin_us` (kernel device-delivery latency, see §2); setting that margin to 0 restores the pure frame-ratio floor for this profile. It represents the absolute physical floor of the game.
* **`balanced`:** The general default profile. Uses `min_hold_frames = 1.02`, adding a small buffer over the host frame boundary to prevent edge-case misses.
* **`audience_safe`:** Recommended for online audience playback. Uses `min_hold_frames = 1.5` — a half-frame cushion that survives lost / late remote frames better than the 1.0–1.05 range that drifts under load. Earlier docs claimed `1.1`; that value was retired in favour of the more conservative `1.5` after remote-room stutter evidence.

### Online Audience Considerations
At high local FPS, a 1-frame hold becomes very short in absolute time (e.g. 6.94 ms at 144 FPS). If online listeners are running at 60 FPS, their clients sample at 16.67 ms intervals and will miss these brief events. Thus, when playing in online rooms, users should use `audience_safe` or calibrate their local FPS to match the audience (typically 60 FPS) to ensure remote registration.

---

## 5. Investigation Findings & Historical Validation

### 2026-06-06 Investigation Summary
1. **Sender Dispatch is Clean:** Extended test sweeps (88 real songs under varying FPS and send durations) resulted in **0 notes dropped** on the sender side (`dropped_conflict`). Note drops only occur on synthetic test cases deliberately authored below the frame duration.
2. **Real Songs Do Not Hit the Same-Key Floor:** The minimum same-key interval across the entire song corpus is **76 ms** (in the song `blue`), with a P50 of ~996 ms. Zero transitions occur below 70 ms. Consequently, same-key floor compression is not a cause of note loss in normal gameplay.
3. **Consistency of Profiles:** Reloading, switching, or persisting profiles results in identical round-trip calibration values (e.g., exactly 6945 $\mu\text{s}$ at 144 FPS), proving that config persistence is robust.
4. **Game FPS Toggle is Not a Workaround:** Early reports suggested toggling the game FPS (e.g. 144 $\rightarrow$ 60 $\rightarrow$ 144) resolved missing notes. Controlled testing showed this is not a reliable fix and likely only resets a volatile game focus/timing state. Missed notes at high FPS are due to game-side sampling phase alignment or runtime thread scheduling delays, not scheduler math.
5. **Hardened Input Path:** Robustness changes include re-acquiring the active game window handle on play, enforcing a 1 ms timer guard in the dispatch thread, and enabling diagnostic startup telemetry under `PLAYBACK_DEBUG`.

---

## 6. Metric Honesty (2026-07-18)

Telemetry metrics are sender-side proxies, **not** game-onset ground truth:

| Metric | Means | Does **not** mean |
|--------|--------|---------|
| `actual_us` | Timeline when backend call began (`T_call_entry`) | Game sampled the key (`T_game_observed`) |
| `send_completed_us` / `dispatch_completed_us` | `perf_counter` after `SendInput` returned (`T_call_return`) | Kernel delivered key; game polled (`T_game_observed`) |
| `visible_lateness_us` | `send_completed_us − scheduled_us` (sender-side completion error) | Game-onset error |
| `observed_hold_us` | Completion-to-completion on sender timeline | Game-visible hold |

The summary JSON includes `timing_semantics.onset_definition = "sendinput_return"` and
`game_observed.available = false` until WASAPI/onset evidence is explicitly attached (Phase J).
Do **not** treat `visible_lateness_us ≈ 0` as proof the game received the note on time.

---

## 7. Accuracy Improvements (2026-07-18 Overhaul)

### Cold-start lead elimination (Phase D)
The `SendLatencyEstimator` now persists its per-kind EMA state to `.cache/lead_estimator.json`
between sessions. On the next play, warm EMA values are imported so the first note benefits from
previous-session lead estimates rather than cold-starting at zero for `_SEED_SAMPLES` (5) sends.
Corrupt or version-mismatched cache is silently ignored — never raises into play.

### Idle-gap core warmup (Phase E)
After a gap of ≥ 20 ms since the last `SendInput` completion elapsed, the dispatch thread
expands its final spin threshold by adding `core_warmup_budget_us` (default 200 µs, capped at
`CORE_WARMUP_SPIN_MAX_US` = 500 µs) to the `effective_spin_threshold`. This widens the busy-spin
window immediately before the deadline — warming the CPU core without a separate blocking
spin. The cold guard is skipped when the dispatch is already late, a command requires pause/stop,
a release is due at the same target, or no blocking interval remains.
Controlled in `core/loop.py` by `core_warmup_budget_us` (default 200) and
`SEND_COLD_THRESHOLD_US` (20_000 µs).

### Mid-song spin re-probe (Phase H)
The pre-play spin probe derives `effective_spin_threshold_us` once before playback. Mid-song,
if a gap of ≥ 0.5 s remains to the next deadline AND ≥ 30 s have elapsed since the last reprobe,
the dispatch thread re-probes timer wake error (8 × 2 ms sleeps) and updates
`spin_threshold_us` with hysteresis (±50 µs). Kill switch: `enable_spin_reprobe = False`
(auto-disabled when `enable_adaptive_spin = False`).

---

## 8. Appendix: Retired Knobs
To clean up the codebase and reduce scheduling overhead, several historical timing knobs were completely removed in June 2026 after empirical testing proved they had no beneficial impact on real playback.

For historical context and audit details of these knobs, refer to the archived documents in [archive/](archive/):
* **`input_lead_us`:** Retired because the player generates its own timeline with no external clock reference. A uniform shift is unobservable. See [timing-architecture-audit.md](archive/2026-06_timing-architecture-audit.md).
* **`chord_merge_window_us`:** Retired because real songs do not contain notes clustered within 5–20 ms; they are either simultaneous or $\ge 100\text{ ms}$ apart. See [timing-experiments.md](archive/2026-06_timing-experiments.md).
* **`frame_align` & `down_only`:** Snapping events to the player's frame grid is useless because the game samples on its own unsynchronized render loop. Snapping introduced offset errors without increasing capture.
* **`release_gap_us` & `repeat_release_gap_us`:** Removed after corpus audits showed they did not bind on real songs and only inflated scheduler complexity. Same-key repeats are now governed purely by the `min_hold_us` constraint.
