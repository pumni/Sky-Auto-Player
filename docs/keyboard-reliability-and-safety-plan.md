# Keyboard Deliverability & Safety Plan

> **Status:** Proposed (not yet implemented). Cross-references: [architecture.md](architecture.md) (input hardening),
> [rt-dispatch-architecture.md](rt-dispatch-architecture.md) (dispatch loop, `command_event`, event waits),
> [timing-principles.md](timing-principles.md) (same-key feasibility floor).
>
> **Governing rules:** [AGENTS.md](../AGENTS.md) is authoritative. This plan stays inside P0: **`SendInput` only** — no
> driver/HID/kernel injection, no memory reads, no game tampering. All work is sender-side and user-mode.

---

## 0. Purpose

Answer one product question precisely: **"Does the game always receive every kind of keypress?"**

Short answer: the *encoding* foundation is already correct and layout-agnostic; what is **not** unconditionally
guaranteed is *delivery*. This plan hardens the three real delivery gaps (focus, partial-send atomicity, and the
frame-quantization ceiling), adds a hard-kill stuck-key failsafe, and reworks control hotkeys to be event-driven and
suppressible. It explicitly drops ideas that are out of scope or violate P0.

---

## 1. Deliverability verdict (why this plan is shaped this way)

### 1.1 Already correct — do not touch

| Press type | Existing mechanism | Verdict |
|---|---|---|
| **Any keyboard layout** (AZERTY / QWERTZ / …) | `scan_code_mode="physical"` sends the **physical scan code** (`layouts.py:PHYSICAL_SCAN_CODES`) | ✅ Already layout-agnostic |
| Chords (simultaneous keys) | Single atomic `SendInput` batch (`inputs.py:_send_scan_code_batch_impl`) | ✅ |
| Sustained/held notes | Duplicate-down suppression (`backend.py:_decide_down`) | ✅ |
| Same-key repeats (tremolo/staccato) | Scheduler `strict`/`degraded` policy + `min_hold_us` + up-gap modelling (`scheduler.py:build_key_actions`) | ✅ Deeply modelled |
| Idempotent release | `release_all()` 3-pass + `GetAsyncKeyState` verification | ✅ |

**Key fact about layout:** a scan code encodes a *physical key position*, not a character. Sending scan `0x15` always
presses the physical key at the QWERTY-`Y` position regardless of the OS keyboard layout. Therefore `"physical"` mode is
the robust, layout-agnostic path; `--scan-code-mode mapped` (`MapVirtualKeyW`) is the *fragile* one for non-QWERTY users.
**No layout refactor is needed.** Default stays `physical`.

### 1.2 Real gaps — where the game may not receive

1. **Focus loss mid-play (highest practical risk).** `SendInput` targets whatever window is foreground *at delivery
   time*. If Sky loses focus, note-ons leak to another window, and note-offs leak too, leaving a **game-side stuck key**.
   There is a check-vs-deliver race between `is_sky_active()` and the actual injection.
2. **Partial chord send.** `SendInput` occasionally returns `sent < n`; the remainder goes in a second call, splitting
   the chord (already instrumented via `_CHORD_SPLIT_EVENTS`, already retried).
3. **Frame-quantization ceiling (hard physical limit).** A ~60fps game cannot distinguish two presses of the same key
   closer than ~16.67ms (one frame). No sender improvement can beat this; we can only *measure and surface* it.

### 1.3 Explicitly out of scope

- **Elevation / UIPI (Sky-as-Admin) mismatch** — the operator confirmed Sky never runs elevated in this environment.
  No preflight check, no counter, no doctor rule. The existing UIPI hint in `send_input_batch`'s error text is left
  as-is.
- **Mouse / camera automation (Bezier smooth-move)** — rejected by product decision; adds no value to a music player.
- **Driver / Interception / Arduino HID** — violates P0 (`SendInput` only) and is anti-cheat evasion. Never.
- **`QueryThreadCycleTime` timer** — cycle time is not wall-clock and is the wrong tool for latency; `perf_counter_ns`
  (QPC) is already optimal.
- **Preemptive chord staggering** — deliberately breaking atomicity to pre-empt a rare OS split is a net regression;
  `THREAD_PRIORITY_TIME_CRITICAL` already exists in `rt_priority.py`.

---

## 2. Workstreams

Three independent workstreams, ordered by safety ROI. Each ships on its own. Every task starts with a failing test that
goes green (AGENTS.md). Test gate for any change: `uv run ruff check . && uv run pyright && uv run pytest`.

### Workstream A — Deliverability hardening (the "always receives" requirement)

#### A0 — Instrumentation first (measure before fixing)
- **Goal:** make the remaining gaps observable, the way `_CHORD_SPLIT_EVENTS` already is.
- **Changes:**
  - `platform/win32/inputs.py`: add counters `_SEND_WHILE_UNFOCUSED` (incremented when a send is attempted while the
    focus guard reports Sky inactive) and surface `min_same_key_up_gap_us` / `impossible_same_key_repeats` (already
    computed in the scheduler) into runtime telemetry.
  - Extend `get_send_diagnostics()` and `BackendHealth` (`backend.py`) with the new fields.
- **Tests:** extend `tests/test_send_diagnostics.py` with count assertions.
- **Risk:** low (counters only). **Effort:** ~0.5 day.

#### A1 — Focus-loss suspend / refocus-and-resync (core of "always receives")
- **Problem:** the check-vs-deliver race; note-offs leaking to the wrong window ⇒ game-side stuck key.
- **Correct model** (this is the subtle part): scan-code injection is *stateless* on our side — we never physically
  hold a key, so `GetAsyncKeyState` shows our note keys as up. A stuck key after focus loss is **purely game-side** and
  can only be cleared by a `release_all()` that actually reaches Sky, i.e. **after** Sky is foreground again. Therefore:
  1. **Never inject while unfocused.** Re-check `is_sky_active()` immediately before each `key_down` batch; if inactive,
     do not emit new note-ons — enter a `suspended` dispatch state.
  2. **On focus loss with in-flight held notes:** do *not* try to release into the wrong window. Suspend, and drive
     refocus via the existing focus guard + `focus_restore_grace_us` knob (INDEX O10.6).
  3. **On focus regain:** issue `release_all()` first (clears any half-held game-side state), then resume the timeline
     from a clean point. Wire this through the existing `command_event` so the supervisor wakes on focus transitions
     (`wait_strategy.wait_until_us` already supports event-driven wake).
- **Files:** dispatch/supervisor layer (`orchestration/dispatch_loop.py`, `orchestration/playback_supervisor.py`),
  reusing `infrastructure/focus.py`. **The pure scheduler is not touched.**
- **Tests:** inject a fake `FocusGuard` whose `is_active()` alternates; assert (a) zero note-ons emitted while inactive,
  (b) `release_all()` fires on regain before resume, (c) no send is attempted into the unfocused state.
- **Risk:** medium — an over-sensitive threshold makes playback stutter. Threshold is configurable, conservative default.
  **Effort:** ~2 days.

#### A2 — Surface the frame-quantization ceiling (honesty, not a fix)
- **Goal:** we cannot beat one game frame; we *can* warn when a song asks us to. The scheduler already computes
  `min_same_key_up_gap_us`, `risky_same_key_repeats`, `impossible_same_key_repeats`.
- **Changes:** surface these on the HUD / schedule-build telemetry: e.g. "N same-key repeats faster than one frame
  @60fps — the game may merge them." Reuse `scripts/audit_same_key_gap.py` as the threshold reference.
- **Tests:** assert the counts propagate into `ScheduleMetadata` and render.
- **Risk:** low (reporting only). **Effort:** ~0.5 day.

### Workstream B — Hard-kill stuck-key failsafe (idea #2, trimmed)

#### B1 — Lightweight watchdog subprocess
- **Problem closed:** `atexit` / `signal` cannot fire on `TerminateProcess` (Task Manager End Task) or a C-level
  segfault, leaving keys physically held in the game.
- **Design (exploits the fixed 15-key alphabet):**
  - The main process spawns one minimal watchdog (`python -m sky_music.watchdog`) and streams a **one-way heartbeat**
    over an anonymous pipe / named event.
  - The watchdog does **not** track which keys are held. On heartbeat loss beyond a threshold (default ~750ms — wider
    than 500ms to avoid false trips on GC/stall) it sends `KEYUP` for **all 15 scan codes** (idempotent; extras are
    harmless) and exits.
  - The watchdog depends only on `platform/win32/inputs.send_scan_code_batch`; it imports no UI/scheduler code, and it
    self-terminates on parent exit (pipe EOF / parent-PID gone).
- **Files:** new `infrastructure/watchdog.py` + entrypoint. Dispatch untouched.
- **Tests:** heartbeat protocol (mock clock); blanket-release hits exactly the 15 scan codes; watchdog self-exits while
  the parent keeps beating.
- **Risk:** medium — subprocess lifecycle. Must guarantee self-death on clean parent exit. **Effort:** ~2–3 days.
- **Note:** the watchdog's `SendInput` reaches whatever is foreground; on a hard-kill Sky is typically still foreground,
  so the blanket release lands. This is a best-effort failsafe, not a guarantee — and strictly better than today.

### Workstream C — Event-driven, suppressible control hotkeys (idea #1)

#### C1 — Tag self-generated input via `dwExtraInfo` (do first, independent)
- **Change:** `inputs.py:_cached_key_input` sets `dwExtraInfo = SKY_PLAYER_SIGNATURE` (a magic constant, e.g.
  `0x5C1B9111`) instead of `0`. No behavioural change today (the game ignores this field), but it is the prerequisite
  for C2's feedback-loop guard.
- **Tests:** assert built `INPUT` structs carry the signature.
- **Risk:** very low. **Effort:** ~0.5 day.

#### C2 — `WH_KEYBOARD_LL` hook for control hotkeys
- **Real value:** not CPU savings — the ability to **suppress** panic/pause/skip so they do **not** leak into the game,
  plus event-driven latency.
- **Design:**
  - A dedicated thread runs a Windows message pump (`GetMessage`/`TranslateMessage`/`DispatchMessage`) and installs
    `SetWindowsHookExW(WH_KEYBOARD_LL)`.
  - The callback is **minimal** (the ~300ms `LowLevelHooksTimeout` budget is unforgiving): check
    `if (info.dwExtraInfo & SIGNATURE) == SIGNATURE: return CallNextHookEx(...)` to **ignore our own injected keys**
    (feedback-loop guard); if the key is a control hotkey, push to a lock-free queue / set a `threading.Event` and
    **return 1** to swallow it; otherwise `CallNextHookEx`. **No logging, no locks, no GIL-heavy work in the callback.**
  - `PlaybackControls.poll()` is replaced by a consumer of that queue; `poll()` remains as a fallback when hook install
    fails.
- **Files:** new `infrastructure/hotkey_hook.py`; `hotkeys.py` keeps parsing/binding. Reuse `tests/probe_postmessage.py`
  for manual/integration probing.
- **Tests:** unit-test the pure parts (scancode→action routing, signature filtering) split from Win32; the hook itself
  is integration/manual.
- **Risk:** highest of the three — a faulty callback gets the hook silently unhooked by Windows; over-eager swallowing
  annoys the user. Ship behind a feature flag, default **off** until proven. **Effort:** ~3–4 days.

---

## 3. Sequencing (by ROI / risk)

1. **A0** (measure) → **C1** (tag `dwExtraInfo`) — small, low-risk, unblock later work.
2. **A1** (focus suspend/refocus-resync) — core of "always receives", medium risk.
3. **A2** (surface frame ceiling) — small, honest.
4. **B1** (watchdog) — high safety value, medium risk.
5. **C2** (LL hook) — high value, highest risk; last, behind a flag.

**Recommendation:** land **A0 first** to get real per-machine numbers on which gap (focus vs partial-send) actually
drops signal before investing in the heavier A1/C2 work.

---

## 4. Validation

| Change scope | Gate |
|---|---|
| Counters / telemetry (A0, A2) | `uv run pytest tests/test_send_diagnostics.py` then full `uv run pytest` |
| Focus dispatch (A1) | fake-focus unit tests + `uv run pyright` + full `uv run pytest` |
| Watchdog (B1) | protocol unit tests + manual hard-kill (Task Manager) verification |
| Hotkey hook (C2) | pure-logic unit tests + manual hook probe; feature-flag off by default |

All workstreams must pass `uv run ruff check . && uv run pyright && uv run pytest` before merge.

---

## 5. Open questions

- **A1 threshold:** what `focus_restore_grace_us` value balances "don't stutter on a transient focus blip" against
  "don't keep dispatching into a lost window"? Tune against O10.6.
- **B1 heartbeat interval / timeout:** 750ms is a starting guess; validate against real GC/stall pauses on the target
  machine so the watchdog never false-trips mid-song.
- **C2 default state:** keep the hook opt-in until the swallow behaviour is validated to never eat a legitimate game key.
