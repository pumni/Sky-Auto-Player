# SendInput Lifecycle Hygiene & Timestamp Fidelity Plan

> **Status:** Implemented (Phases 0–4 shipped in tree). Residual: Phase 5 doctor preflight
> (Sky-foreground-before-start not yet shipped — §5.1) and final doc polish; Phase 6 WASAPI
> validation optional and non-blocking (§6). See §2.4 “As-built status snapshot (2026-07-16)”
> and §11 decision log (as-built rows) for divergences with intent. Date: 2026-07-16.
>
> **Cross-references (canonical contracts — do not contradict without updating them):**
> - [AGENTS.md](../AGENTS.md) — P0: SendInput only; no game memory / injection / anti-cheat bypass
> - [timing-principles.md](timing-principles.md) — completion-anchor, min_hold floor, adaptive lead
> - [rt-dispatch-architecture.md](rt-dispatch-architecture.md) — DispatchLoop, lead, wait strategy
> - [architecture.md](architecture.md) — layering. §3 dual-release model is now in sync with
>   the runtime (Phases 1–2 delivered); see architecture.md §3 footnote for the pre-Phase-1
>   doc-debt acknowledgement.
>
> **Relation to prior plans:**
> - Supersedes the *focus-release strategy* and *partial-send description* in
>   [archive/keyboard-reliability-and-safety-plan.md](archive/keyboard-reliability-and-safety-plan.md)
>   where they diverge from current code or from the dual-release model below.
> - Does **not** replace [rt-dispatch-architecture.md](rt-dispatch-architecture.md) lead/spin design;
>   it hardens the *input-state lifecycle around* that design and tightens *when* SendInput runs.
> - Complements [archive/2026-06_wasapi-loopback-measurement-plan.md](archive/2026-06_wasapi-loopback-measurement-plan.md)
>   for after-send ground truth (Phase F).
> - Out of scope: Rust migration ([rust-migration-plan.md](rust-migration-plan.md)), RAM hygiene,
>   UI polish, profile formula rewrites.

---

## 0. Purpose & product question

**Product question:** When the user hits Play, does every intended note-on / note-off reach Sky
via Windows `SendInput` with **correct pairing**, **safe keyboard state**, and **completion
timestamps as close as possible to the authored schedule**?

**Short answer today:** The *encoding* of `KEYBDINPUT` (scan code, flags, atomic chords) is
already correct. What is **not** fully best-practice is:

1. **Lifecycle of held keys** across focus loss / pause / panic / exit (asymmetric abort paths).
2. **Gate before note-on** (focus check-vs-send race).
3. **Observability** of partial SendInput and unfocused attempts.
4. **Policy defaults** so “timestamp fidelity” (onset ≈ `scheduled_us` at SendInput *completion*)
   is the production default, not an expert flag.

This plan is a staged hardening + small logic fix pass. It is **not** a scheduler rewrite and
**not** a change to the mathematical meaning of `min_hold_us` / completion-anchor.

---

## 1. Ground rules (frozen across all phases)

| # | Rule |
|---|------|
| G1 | **SendInput only.** No `PostMessage`/`SendMessage` key injection, no drivers, no HID, no game memory. |
| G2 | **Scan-code path remains default.** `KEYEVENTF_SCANCODE`, `wVk = 0`, `time = 0`, physical 15-key map. |
| G3 | **Pure AOT scheduler unchanged in golden meaning.** `build_key_actions` golden snapshots must stay green unless a phase *explicitly* documents a metadata-only additive field. |
| G4 | **Completion-anchor stays.** `release_not_before = down_dispatch_completed + min_hold`. Floor always wins over lead. |
| G5 | **Musical note-on partial policy stays.** If `SendInput` returns `sent < n` on note-on: **do not** complete the remainder late. Drop tail (`DROPPED_BACKEND`). Note-off / panic: **do** complete remainder. |
| G6 | **Exact simultaneous chords** stay one `SendInput` batch when `chord_stagger_us == 0`. |
| G7 | **Dispatch thread owns all backend sends** (existing threaded contract). Abort helpers may only be called from that thread or after join. |
| G8 | Every phase: failing test first → green; gate `uv run ruff check . && uv run pyright && uv run pytest`. |

---

## 2. Production reality baseline (code truth, pre-Phase-1 — 2026-07-15)

> The subsections below describe the **baseline** as of the plan's authoring date (2026-07-15,
> before any phase shipped). They are preserved unchanged as the historical gap analysis that
> justified Phases 0–6. Phases 0–4 have since shipped — see §2.4 “As-built status snapshot”
> for the current state and which baseline gaps are closed/residual.

### 2.1 What was already best-practice (do not regress)

| Area | Location | Verdict |
|------|----------|---------|
| Scan-code `SendInput` | `platform/win32/inputs.py` | ✅ Correct for game physical keys |
| INPUT cache + prewarm | same | ✅ Thin hot path |
| Note-on no-retry / note-off complete | `_send_scan_code_batch_impl` | ✅ Correct musical + safety split |
| Duplicate-down / idempotent-up | `infrastructure/backend.py` | ✅ |
| Completion-anchor + no-early-conflict | `runtime_dispatch.py` | ✅ |
| Adaptive lead (onset = completion) | `engine.py` `SendLatencyEstimator` | ✅ Sender-side timestamp fidelity |
| Watchdog full 15-key KEYUP | `watchdog.py` | ✅ Hard-kill failsafe (already present) |
| Panic / end `release_all` | `dispatch_loop` finally | ✅ |

### 2.2 Baseline gaps this plan set out to close

> Status legend: ✅ closed by Phase 0–4 ship; 🟡 partially closed / divergence (see §11 as-built
> decisions); ❌ residual (Phase 5 / 6). Each row's `Evidence` cell below is still the **baseline**
> evidence cited when the plan was authored; the as-built status column tracks current code.

| ID | Gap | Evidence in code (baseline) | Impact | As-built status (2026-07-16) |
|----|-----|------------------|--------|------------------------------|
| **L1** | Focus-loss does **not** call `release_all` immediately; only `cancel_all` | `dispatch_loop.py` `_process_wait_states`: focus lost → `cancel_all()`; `release_all()` only on restore | System key state can stay down while user is in another app; coordinator/backend state diverge until restore; docs (`architecture.md`) claim the opposite | ✅ Closed — `dispatch_loop.py:858,892`: focus-lost now `_abort_input_safe("focus_lost")` (release-first); regain issues a second `release_all()` before resume |
| **L2** | Abort paths are **asymmetric** | Manual pause / panic / finally: `release_all`+`cancel_all`; focus lost: cancel only | Harder to reason; easy to reintroduce stuck-key bugs | ✅ Closed — single `_abort_input_safe` used by pause/panic/focus-lost/finally (`dispatch_loop.py:334,381,564,795,806,858,1217`) |
| **L3** | Focus check-vs-send race | `DispatchHealthMonitor` focus cache TTL 2 ms; supervisor poll 20–50 ms; no recheck immediately before `key_down` | Possible note-on after Sky lost focus → wrong consumer + game may miss note-off later | 🟡 Closed with divergence — pre-down gate implemented via runtime `FocusSignal` + `_first_down_dispatched` arm (`dispatch_loop.py:559-573,1127-1162`), not `focus_is_active(force=True)` TTL=0. Race is vs last control-thread focus sample, not live OS foreground — see §11 as-built row 1 |
| **L4** | Partial note-on is correct inject-wise but soft on outcome labeling | Backend returns `success=False` / prefix; runtime may still look “sent” in coarse counters | Operators cannot tell chord was truncated | ✅ Closed — runtime outcomes `partial_note_on` (`dispatch_loop.py:618,624`); summary `partial_note_on_count` (`telemetry.py:841`) |
| **L5** | Architecture / keyboard plan drift | `architecture.md` §3; archived keyboard plan assumes “never KEYUP unfocused” and “partial remainder retried” | Implementers follow docs → wrong fix | ✅ Closed — `architecture.md:64,65,70-72` documents dual-release + acknowledges pre-Phase-1 doc debt; `INDEX.md:27,41` stamps archive keyboard plan as partially superseded; this plan's status line + §11 now match code |
| **T1** | Timestamp fidelity depends on adaptive lead ON + warm estimator | Cold 5 samples lead=0; first notes systematic late → floor defers releases | Early notes worse than mid-song; worse without lead cache | ✅ Verified — lead cache path exercised by `tests/test_adaptive_lead.py` (round-trip / DryRun no-write / poison rejection); cold-start covered |
| **T2** | `visible_lateness≈0` is **sender** fidelity, not game sample phase | No phase lock with game frames (by design) | 1.0-frame `local_precise` still probabilistic miss | ✅ Documented as design fact — no claim of game phase lock in this plan; Phase 6 WASAPI is optional validation only |
| **T3** | Unfocused send counter not driven from the real gate | `_SEND_WHILE_UNFOCUSED` comment says not incremented on hot path consistently with block | Weak diagnostics | 🟡 Partially closed — `note_send_while_unfocused()` increments at `dispatch_loop.py:467-470` when `require_focus and not focus_is_active()`; after Phase 2 gate, musical downs while unfocused are rare. Counter is post-gate (downs that did execute), not strictly “attempted but blocked” — see §11 as-built row 4 |

### 2.3 Dual nature of “stuck keys” (design fact)

`SendInput` injects into the **system keyboard input stream**. Consumption is focus-routed:

| Layer | What “key down” means | How to clear |
|-------|----------------------|--------------|
| **OS keyboard state** | Async key state / stream after our downs | KEYUP via `SendInput` **anytime** (updates OS state) |
| **Game-side logical hold** | Sky sampled a down while focused and still thinks held | KEYUP that Sky **actually consumes** → needs Sky foreground (or next focus sample) |

Therefore the correct focus model is **dual-release**, not “never release unfocused” and not “release only unfocused”:

```text
Focus LOST  → abort_input_safe(): KEYUP all tracked (+ optional full 15)
              + cancel generations + freeze timeline
              → clears OS state; may or may not clear game-side hold

Focus REGAINED → KEYUP again (idempotent) while Sky is foreground
              → clears game-side half-holds
              → grace → resume timeline (cursor continues; mid-note gens already cancelled)
```

This supersedes archive keyboard plan A1 step 2 (“do not release on loss”).

### 2.4 As-built status snapshot (2026-07-16)

> Read this section *together with* §11 (decision log — as-built rows). §2.1–§2.3 are preserved
> as the pre-Phase-1 baseline; this subsection is the current truth.

| Phase | Plan intent | As-built status | Anchor in tree |
|-------|-------------|-----------------|----------------|
| 0 | Counters + doc truth + A/B flags placeholder | Shipped (counters, summary keys). A/B flags `release_all_on_focus_*` **not** added — dual-release hard-wired, see §11 row 3. | `telemetry.py:269,841,452,836`; `dispatch_loop.py:467-470`; `inputs.note_send_while_unfocused()` |
| 1 | `_abort_input_safe` + focus dual-release | Shipped. Release-first, then cancel; regain re-`release_all()` (no re-cancel). | `dispatch_loop.py:334-372,858-861,892-901`; tests `test_focus_input_lifecycle.py` |
| 2 | Pre-down focus recheck gate | Shipped with divergence — uses `FocusSignal` cached at `run()` entry + `_first_down_dispatched` arming, not `focus_is_active(force=True)` TTL=0. Race vs last control-thread focus sample, not live `GetForegroundWindow`. | `dispatch_loop.py:559-573,1127-1162`; tests in `test_focus_input_lifecycle.py` |
| 3 | Partial-send outcome hygiene | Shipped. `runtime_outcome ∈ {"partial_note_on"}` for strict-prefix note-on; no late retry (G5 honored). | `dispatch_loop.py:618,624`; `telemetry.py:841` (`partial_note_on_count`) |
| 4 | Timestamp fidelity + cold-start | Shipped (audit + tests). Lead cache round-trip, DryRun no-write, poison rejection covered. | `tests/test_adaptive_lead.py` |
| 5 | Doctor preflight + final doc sync | **Partial.** architecture.md + INDEX.md in sync; doctor `check_sky_window()` checks Sky **window found** but not **foreground before start** — §5.1 residual. | `doctor.py:27-54`; `architecture.md:64,65,70-72`; `INDEX.md:27,41` |
| 6 | Optional WASAPI validation gate | Not shipped (optional / non-blocking, consistent with this plan). | — |

**Residual work to fully close this plan:** (a) Phase 5 doctor `require_focus` foreground
preflight (small, scoped PR — does **not** require re-opening Phases 0–4); (b) optional Phase 6
WASAPI measurement; (c) keep §11 decision log accurate as further divergences appear.

**Do not re-litigate closed gaps.** L1, L2, L4, L5, T1, T2, and the Phase 0 counters are closed.
Implementers reading this plan should treat §2.2's "Evidence in code (baseline)" column as
historical context only — fixing those exact symptoms in the current tree would regress the
as-built contracts above.

---

## 3. Target end-state contracts

### 3.1 Input abort contract (`abort_input_safe`)

Single helper used by **all** interrupt paths on the dispatch thread:

| Step | Action |
|------|--------|
| 1 | `backend.release_all()` — multi-pass KEYUP of `active ∪ possibly_active ∪ failed_release` |
| 2 | Optional **full Sky-15 KEYUP** (idempotent; same set as watchdog) when `panic_full_keyboard=True` (default **true** for panic / process teardown; **false** for normal pause if product wants quieter abort — default recommendation: **true** for focus-lost and panic, **tracked-only** for short manual pause is acceptable if tests prove tracked set complete) |
| 3 | `coordinator.cancel_all()` — terminalize ACTIVE / RELEASE_PENDING |
| 4 | Clear or refresh health snapshot; record telemetry reason: `manual_pause` \| `focus_lost` \| `panic` \| `quit` \| `finished` \| `error` |

**Invariant after abort:** no scan code in backend tracking sets is considered held; no non-terminal live generation remains active/pending.

### 3.2 Note-on gate contract

Before every **musical** `key_down` (not panic KEYUP):

```text
if require_focus:
    if not focus_is_active_fresh():   # bypass or refresh TTL cache
        enter focus-pause via abort_input_safe(reason=focus_lost)
        do not call SendInput for this note-on
```

KEYUP paths (scheduled release, abort, watchdog) are **never** blocked by focus gate
(scheduled release while unfocused is already avoided by timeline freeze; abort KEYUP must run).

### 3.3 SendInput encoding contract (unchanged, reaffirmed)

| Field | Value |
|-------|--------|
| `type` | `INPUT_KEYBOARD` |
| `wVk` | `0` |
| `wScan` | physical scan code |
| `dwFlags` | `KEYEVENTF_SCANCODE` [\| `KEYEVENTF_KEYUP`] |
| `time` | `0` |
| `dwExtraInfo` | `SKY_PLAYER_SIGNATURE` (0x5C1B9111) |

Chord: one `SendInput(n, array, cbSize)` for n keys when stagger off.

### 3.4 Timestamp fidelity contract (sender-side)

| Term | Definition | Production target |
|------|------------|-------------------|
| Authored onset | `KeyAction.at_us` / batch `scheduled_us` | Unchanged AOT |
| Dispatch completion | `send_completed_us` mapped into playback elapsed | **Onset truth for lead/floor** |
| `visible_lateness_us` | `completion_elapsed - scheduled_us` | p50 ≈ 0 ± few 100 µs after warm lead |
| Hold floor | `down_completed + min_hold` | Never violated for released gens |
| Game sample | Unknown phase; ≥ 1 frame hold required | Policy via profiles, not phase snap |

**Fidelity stack (keep / enable in production defaults):**

1. Adaptive lead ON (completion-targeted pop).
2. Lead residual prologue bias ON (already in estimator).
3. Lead cache warm-start ON for real backend.
4. Completion timestamp stamped **immediately** after `SendInput` return (no telemetry between).
5. Floor always wins.
6. No-early-conflict guard (no lead-induced drops).

---

## 4. Workstreams & phases

Phases are **sequentially ordered** for dependency, but each must ship with green full suite and be revertible alone.

```text
Phase 0  Instrumentation & doc truth
Phase 1  abort_input_safe + focus dual-release          ← core lifecycle
Phase 2  Note-on focus recheck gate
Phase 3  Partial-send / outcome hygiene
Phase 4  Timestamp fidelity defaults & cold-start
Phase 5  Preflight (doctor) & architecture doc sync
Phase 6  Optional measurement (WASAPI / in-game) gate
```

---

### Phase 0 — Instrumentation & documentation truth

**Goal:** Measure and stop lying in docs before behavior changes.

#### 0.1 Code / telemetry

- Ensure `send_while_unfocused` increments **only** when a musical note-on would have been / was attempted without active focus (align `note_send_while_unfocused` with the real gate after Phase 2; until then, increment from dispatch path when `require_focus and not focus_is_active()` at `_execute_action` for downs).
- Add telemetry field `abort_reason` / counter map: `abort_counts_by_reason` on session summary (additive).
- Record `release_all_on_focus_lost: bool` and `release_all_on_focus_regain: bool` in `runtime_options` once Phase 1 lands (flags for A/B).

#### 0.2 Docs

- Patch [architecture.md](architecture.md) §3 to describe **current** code until Phase 1 ships, then describe dual-release.
- Note in this plan’s status line when each phase completes.
- Do **not** delete archive keyboard plan; stamp it “partially superseded by this document §2.3 / Phase 1”.

#### Tests

- Diagnostic counters present in `get_send_diagnostics` / summary JSON keys (extend `tests/test_send_diagnostics.py`).

#### Gate

- No behavior change required for merge if only docs + counters; counters must not allocate on non-debug paths beyond int increments.

#### Effort

- ~0.5–1 day.

---

### Phase 1 — Unified abort + focus dual-release (**P0 lifecycle**)

**Goal:** One abort contract; focus-loss hygiene matches pause/panic safety.

#### 1.1 Implementation

**New helper** (suggested name/location):

- `DispatchLoop._abort_input_safe(self, reason: str) -> ReleaseAllOutcome`
  - Calls `backend.release_all()` then `coordinator.cancel_all()` (order: **release first**, then cancel — so tracking sets still know what to up).
  - Today pause uses `_release_all_and_cancel_runtime` which already does this order — **rename/alias** and route all paths through it.

**Focus-lost path** (`_process_wait_states` when `require_focus and not focus_signal.is_active()`):

```text
if focus_pause_started_us is None:
    outcome = self._abort_input_safe("focus_lost")   # NEW: includes release_all
    state.focus_pause_started_us = clock.now_us()
    telemetry.record_abort / record_release_outcome (additive)
# freeze timeline as today (pause_time accounting)
```

**Focus-regain path** (after grace, existing block):

```text
# Keep release_all HERE as second, game-facing clear (idempotent)
self.backend.release_all()
# do NOT cancel_all again in a way that double-counts terminals incorrectly;
# generations already cancelled on loss. release_all alone is enough.
update pause_time; clear focus_pause_started_us
```

**Manual pause / panic / finally:** call the same `_abort_input_safe` (panic may pass `full_keyboard=True`).

**Optional full-15 KEYUP:** implement as `backend.release_all(full_instrument=True)` or post-step in abort for focus_lost + panic using `PHYSICAL_SCAN_CODES` values (same as watchdog). Default: enable for `focus_lost` and `panic`.

#### 1.2 Explicit non-goals

- Do not rewind `coordinator.cursor` (timeline continues after unpause; mid-hold notes stay cancelled — same as today after cancel).
- Do not change `focus_restore_grace_us` formula (O10.6 remains open experiment).

#### 1.3 Tests (required)

| Test | Assert |
|------|--------|
| Focus lost mid-hold | `release_all` / KEYUP history includes held scan codes **before** regain |
| Focus lost | `active_keys` empty after abort; generations cancelled |
| Focus regain | Second `release_all` (or KEYUP batch) occurs before any new note-on |
| Manual pause | Still releases (regression) |
| Fake clock focus toggle | Zero note-ons while inactive (`require_focus=True`) |
| Dual-release idempotent | Double KEYUP does not throw; backend ends empty |

Suggested file: `tests/test_focus_input_lifecycle.py` (new) using existing fake backend / DryRunBackend patterns from `test_engine_refactor.py` / `test_reprobe_pause.py`.

#### 1.4 Risks & mitigations

| Risk | Mitigation |
|------|------------|
| KEYUP while unfocused “wasted” for game | Dual-release: regain KEYUP still required |
| Extra KEYUP cost / glitch | Idempotent ups; only on transitions |
| Tests depending on “cancel without release” | Update those tests to new contract |

#### Effort

- ~1–2 days.

---

### Phase 2 — Fresh focus recheck before note-on (**race close**)

**Goal:** Eliminate check-vs-send race for musical downs.

#### 2.1 Implementation

In `DispatchLoop._dispatch_down_batch` (or `_execute_action` when `kind==down` and `require_focus`):

1. Call `health_monitor.focus_is_active()` with **force refresh** (new API: `focus_is_active(force=True)` ignoring TTL), **or** set TTL=0 for the pre-send check only.
2. If inactive: do **not** send; trigger same path as focus loss (`_abort_input_safe` + set `focus_pause_started_us` if not set); return `None` / record `runtime_outcome="blocked_unfocused"`.
3. Keep TTL cache for non-critical HUD/health paths.

**Do not** recheck before KEYUP abort (must proceed).

#### 2.2 Tests

- Focus flips false between “deadline wake” and down dispatch → **no** `key_down` in backend history; abort recorded.
- Performance: force refresh only on downs (not on every wait spin).

#### Effort

- ~0.5–1 day.

---

### Phase 3 — Partial-send & runtime outcome hygiene

**Goal:** Keep inject policy; make failures first-class in runtime/telemetry/HUD.

#### 3.1 Implementation

- When `InputSendResult.success` is false on note-on (partial or empty):
  - `runtime_outcome` = `partial_note_on` or keep `sent` with explicit `dropped_backend` already on unsent gens (already in `activate_sent_downs`).
  - Ensure telemetry summary increments `partial_note_on_count` / exposes `keys_dropped` (BackendHealth already has `keys_dropped` — wire to HUD if not visible mid-play).
- Never change note-on to late-retry remainder (G5).
- Document in `get_send_diagnostics` comment block the musical vs safety policy (already in inputs.py — keep in sync with architecture).

#### 3.2 Tests

- Mock `SendInput` returns `sent = n-1` → only prefix active; tail `DROPPED_BACKEND`; no second note-on SendInput for tail.
- Release path still completes remainder (existing tests + strengthen).

#### Effort

- ~0.5–1 day.

---

### Phase 4 — Timestamp fidelity (production defaults & cold-start)

**Goal:** Maximize probability that **SendInput completion** lands on `scheduled_us` under real defaults — without claiming game phase lock.

#### 4.1 Keep (already correct) — verify production wiring

| Knob | Expected production default |
|------|----------------------------|
| `enable_adaptive_lead` | `true` |
| `enable_adaptive_spin` | `true` |
| Lead cache path | set for real backend sessions |
| `dispatch_lead_us` manual override | `0` (use estimator) unless debug |
| Completion stamp | immediately after `SendInput` in `_emit` |

**Audit task (no design change if already true):** grepping console + Textual play paths confirms lead cache path is passed; DryRun never writes cache (already guarded).

#### 4.2 Cold-start hardening (small code if gaps found)

| Item | Action |
|------|--------|
| Lead cache import | Fail soft on corrupt cache (already); add test for poison rejection |
| First-chord polyphony | RLS linear warm-start already exists — add regression test “first N=3 chord lead > 0 after linear seed from singles” if not covered |
| Residual bias | Keep positive-only cap 500 µs — document in rt-dispatch if missing |

#### 4.3 Policy guidance (docs / defaults — not forced profile rename)

| Scenario | Recommended profile | Rationale |
|----------|---------------------|-----------|
| Local practice, high FPS, accept miss risk | `local_precise` (1.0 frame) | Sharp; zero margin |
| Default local | `balanced` | Thin margin over 1 frame |
| Online audience | `audience_safe` | Longer hold for remote sample |

**Explicit:** fixing “game always hears 1.0-frame notes” is **impossible** without phase lock or longer hold; this phase optimizes **sender completion fidelity**, not physics of sampling.

#### 4.4 Optional micro-hardening (only if measurement shows need)

| Idea | Accept if | Reject if |
|------|-----------|-----------|
| Slightly higher default spin floor under load | reprobe evidence | Increases idle CPU without p99 win |
| Cap bookkeeping after send more aggressively | telemetry shows bookkeeping ≫ pure send | Already pure-send for lead |
| Epoch rebase on focus regain | large pause skew issues | Scope creep; separate plan exists in archive |

#### Tests / gate

- Existing adaptive lead tests stay green.
- New: lead cache not written by DryRun; import rejects absurd values.
- Optional: synthetic timeline asserts p50 `visible_lateness` under FakeClock + fixed send duration ≈ 0 when lead enabled after seed.

#### Effort

- ~1 day audit + tests; more only if gaps found.

---

### Phase 5 — Preflight & doc synchronization

**Goal:** Fail early; keep hierarchy of truth honest.

#### 5.1 Doctor / play-start checks (best-effort, non-blocking warn or hard fail flag)

| Check | Action | As-built status (2026-07-16) |
|-------|--------|------------------------------|
| Sky window found | Existing | ✅ Shipped — `doctor.check_sky_window()` (`doctor.py:27-54`) finds HWND, resolves PID + process name, emits UIPI admin-integrity note. |
| Sky foreground before start (if require_focus) | Warn / block start | ❌ **Not shipped** — current `check_sky_window` validates the window exists but does not probe foreground state. This is the principal residual PR of this plan; it does not gate any Phase 0–4 contract and can ship independently. Recommended impl: add a foreground probe (reuse `focus_guard.is_active()` or `user32.GetForegroundWindow() == sky_hwnd`) gated by `require_focus`, emit warn (non-blocking) by default. |
| Optional: process integrity vs self (UIPI) | **Warn only** if easy; do not require admin elevation of player by default | ✅ Shipped — `is_admin()` warning at `doctor.py:43-51` (warn-only, does not mandate elevation). |
| Physically held note keys | Existing doctor held-key warning | ✅ Shipped (existing doctor held-key check). |

#### 5.2 Docs to update when phases complete

| Doc | Update |
|-----|--------|
| [architecture.md](architecture.md) | Dual-release; abort helper; note-on gate |
| [rt-dispatch-architecture.md](rt-dispatch-architecture.md) | Cross-link lifecycle abort; focus gate |
| [timing-principles.md](timing-principles.md) | Clarify sender fidelity vs game sample; fix any profile frame numbers that drift from `config.py` |
| [INDEX.md](INDEX.md) | This plan as active; archive keyboard plan note |

#### Effort

- ~0.5–1 day.

---

### Phase 6 — After-send ground truth (optional validation gate)

**Goal:** Prove game-facing result, not only sender telemetry.

- Prefer implementing / finishing [archive/2026-06_wasapi-loopback-measurement-plan.md](archive/2026-06_wasapi-loopback-measurement-plan.md).
- Compare audio onsets to `scheduled_us` + `visible_lateness` for a short fixture song at `balanced` @ declared FPS.
- Success criteria (suggested): median |onset_error| within 1 frame; no stuck keys after focus toggle script.

This phase does **not** block Phases 1–2 (safety).

#### Effort

- Measurement harness dependent; treat as validation, not production feature.

---

## 5. File touch map (expected)

| Phase | Primary files |
|-------|----------------|
| 0 | `inputs.py` (counters), `telemetry.py`, `architecture.md`, this plan status |
| 1 | `dispatch_loop.py`, possibly `backend.py` (`full_instrument`), `tests/test_focus_input_lifecycle.py` |
| 2 | `dispatch_loop.py`, `DispatchHealthMonitor.focus_is_active(force=...)` |
| 3 | `dispatch_loop.py`, `telemetry.py`, HUD consumer if any |
| 4 | `engine.py` / play entrypoints audit, tests for lead cache |
| 5 | `doctor.py` / CLI doctor, docs |
| 6 | measurement scripts / tests only |

**Must not touch without separate explicit decision:** `domain/scheduler.py` hold math, golden schedules, profile frame ratios in `config.py` (except documented default recommendation text).

---

## 6. Testing matrix

| Layer | What |
|-------|------|
| Unit | abort helper; dual-release order; focus recheck blocks down; partial send prefix |
| Engine fake clock | Full play with focus flip mid-song; pause/panic regression |
| Backend DryRun | History order: down… abort ups… no downs while suspended… regain ups… downs |
| Real machine (manual) | Alt-tab mid-song: no stuck keys in Notepad; after refocus, music continues cleanly |
| Telemetry | `abort_counts`, `keys_dropped`, `send_while_unfocused`, lead stats |

---

## 7. Success criteria (plan done)

1. **Lifecycle:** Every of {manual pause, focus lost, panic, normal finish, exception finally} leaves backend tracking empty and no live ACTIVE/RELEASE_PENDING gens (except finished RELEASED counts).
2. **Dual-release:** Focus lost issues KEYUP; focus regain issues KEYUP again before note-ons; proven by tests.
3. **Gate:** No musical note-on while `require_focus` and Sky inactive. As-built narrowing
   (Phase 2 divergence, see §11 row 1): the pre-down gate consults the **runtime `FocusSignal`**
   (in threaded mode, a `SharedFocusSignal` last sampled by the control thread), not a live
   `GetForegroundWindow` call on the dispatch hot path. The race window is therefore vs the
   *last control-thread focus sample*, not vs the OS foreground at the instant of `SendInput`.
   This is the explicit success bar; making it tighter would require either a synchronous
   foreground probe per down (rejected — hot-path cost) or Phase 6 measurement evidence.
4. **SendInput policy:** Note-on partial still no late retry; note-off still completes; G1–G8 hold.
5. **Timestamp:** Production path keeps adaptive lead + completion stamp; cold-start cache verified; no regression in adaptive lead A/B-class unit tests.
6. **Docs:** `architecture.md` and this plan match code; INDEX points here; archive keyboard plan stamped superseded where needed.
7. **Gates green:** `uv run ruff check . && uv run pyright && uv run pytest`.

---

## 8. Explicitly out of scope

| Item | Why |
|------|-----|
| Kernel/HID/Interception | P0 violation |
| Frame-align bot clock to game render | Unsynchronized; retired earlier for good reason |
| Late retry of partial chord note-ons | Breaks timing / creates ghost notes |
| Process `REALTIME_PRIORITY_CLASS` | OS stability; Microsoft discourage |
| Changing `min_hold` formula / removing completion-anchor | Separate timing program; already settled |
| Rust hot path | Own plan |
| Mouse / camera | Product reject |
| Mandatory elevation of Sky Player | Security / UX; optional warn only |

---

## 9. Implementation order (checklist)

Use as PR sequence (Graphite/stack or sequential PRs). Ticks below reflect as-built status
(2026-07-16); boxes left unchecked are residual work.

- [x] **PR0** — Phase 0 counters + architecture “current truth” note.
  *As-built:* counters shipped (`abort_counts_by_reason`, `note_send_while_unfocused`,
  `partial_note_on_count`, summary keys). A/B flags `release_all_on_focus_*` **not** added —
  superseded by hard-wired dual-release decision (§11 row 3). architecture.md now matches code
  (no provisional "current truth" patch needed; §3 documents the shipped dual-release directly).
- [x] **PR1** — Phase 1 `abort_input_safe` + focus dual-release + tests.
  *As-built:* `_abort_input_safe` is the single abort entrypoint (`dispatch_loop.py:334-372`);
  focus-lost calls it (`:858`); regain re-`release_all()` without re-`cancel_all()` (`:892`);
  panic passes `full_instrument=True` (`:795`). Tests in `tests/test_focus_input_lifecycle.py`.
- [x] **PR2** — Phase 2 pre-down focus force-refresh gate + tests.
  *As-built with divergence (§11 row 1):* gate uses `_first_down_dispatched` arming + runtime
  `FocusSignal` (`:559-573,1127-1162`) instead of `focus_is_active(force=True)` TTL=0. Blocked
  downs are labelled `blocked_unfocused` (`:573`) and abort the active hold. Race scope is vs
  the control-thread focus sample, not live OS foreground (see §7 criterion 3 narrowing).
- [x] **PR3** — Phase 3 partial outcome / HUD / summary.
  *As-built:* `partial_note_on` runtime outcome (`:618,624`); `partial_note_on_count` exposed in
  summary (`telemetry.py:841`). G5 (no late retry) honored by the unchanged inject layer
  (`inputs._send_scan_code_batch_impl`).
- [x] **PR4** — Phase 4 lead-cache / cold-start audit + tests.
  *As-built:* audit complete; `tests/test_adaptive_lead.py` covers cache round-trip, DryRun
  no-write, and poison/corrupt rejection. No production-default change was required — required
  knobs were already wired (adaptive lead ON, completion stamp immediately after `SendInput`).
- [ ] **PR5** — Phase 5 doctor + final doc sync (timing-principles profile numbers if drifted).
  *Partial:* architecture.md + INDEX.md already synced (§3 dual-release, INDEX "implemented"
  note still says "Primary active" — minor polish). Doctor `require_focus` foreground preflight
  (§5.1) **not yet shipped** — this is the principal residual PR.
- [ ] **PR6** — Phase 6 optional WASAPI validation (non-blocking).

Each PR: description links this plan phase ID; no drive-by refactors. As-built PRs may map to
multiple commits; the boxes above track contract delivery, not strictly one-PR-per-box.

---

## 10. Rollback

| Phase | Rollback |
|-------|----------|
| 0 | Revert counters/docs |
| 1 | Restore focus-lost `cancel_all`-only; keep pause path |
| 2 | Remove force recheck; rely on pause poll only |
| 3 | Drop new outcome labels; keep inject policy |
| 4 | Revert default/wiring only |
| 5–6 | Docs/measurement only |

---

## 11. Decision log (locked for this plan)

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Focus-loss KEYUP | **Yes, immediately** + KEYUP again on regain | OS hygiene + game-side clear |
| Partial note-on | **No late retry** | Musical atomicity / no ghost notes |
| Scheduler rewrite | **No** | Gaps are lifecycle + gate + defaults |
| Timestamp goal | **Sender completion ≈ schedule** | Only controllable layer under SendInput-only |
| Game 100% hear guarantee | **Not claimed** | Frame sample phase + remote net are outside control |

### 11.1 As-built decisions (added 2026-07-16; supersede intent above where they diverge)

These rows record divergences the shipped code took from the original plan text, with
rationale. They are **not** plan amendments — they document what shipped and why. Future
implementers must treat them as binding contracts unless reopened via a successor plan.

| # | As-built decision | Choice shipped | Rationale | Anchor / location |
|---|-------------------|-----------------|-----------|-------------------|
| 1 | Phase 2 gate implementation | Runtime `FocusSignal` + `_first_down_dispatched` arming (not `focus_is_active(force=True)` TTL=0). Race window is vs last control-thread focus sample, not live `GetForegroundWindow`. | Avoids a synchronous foreground syscall on every dispatch down hot path; `SharedFocusSignal` is lock-protected and updated by the control thread's periodic poll. Tighter race scope than pre-Phase-2 without paying hot-path cost; success criterion #3 is met *under this narrow reading* (see §7 criterion 3 narrowing). To make it tighter would require Phase 6 measurement or per-down foreground probe (rejected). | `dispatch_loop.py:559-573,1127-1162` |
| 2 | `full_instrument=True` scope | Limited to **panic** and process teardown (quit/finished/error via `_abort_input_safe(final_abort_reason)` — final release only; current code only passes `full_instrument=True` at `:795` for panic). **Not** used for `focus_lost` or manual pause, despite Phase 1 §3.1 suggesting "default true for focus_lost". | Tracked-set (backend `active ∪ possibly_active ∪ failed_release`) is the authoritative release set; full-15 KEYUP on every focus flip is extra OS traffic with no proven marginal benefit given tracked-set completeness. Watchdog (`watchdog.panic_release_all`) remains the last-resort failsafe. Reopen only if a real-world stuck-key case is traced to an untracked key under focus transition (none observed). | `dispatch_loop.py:795`; `watchdog.panic_release_all` |
| 3 | A/B flags `release_all_on_focus_lost` / `release_all_on_focus_regain` | **Not shipped (wontfix).** Dual-release is hard-wired; no runtime flag toggles either half off. | The two releases serve orthogonal purposes (OS state clear on loss; game-side half-hold clear on regain). There is no product scenario where disabling one half is correct; an A/B knob would invite misconfiguration and add surface area to the runtime options struct for no real benefit. If a measured regression ever needs a kill switch, reintroduce a single `focus_dual_release_enabled` bool rather than two independent flags. | `dispatch_loop.py:858-901` (no flag, unconditional); `telemetry.runtime_options` |
| 4 | `note_send_while_unfocused` semantics | Increments **post-send** on the dispatch path when `require_focus and not focus_is_active()` (`dispatch_loop.py:467-470`), i.e. counts downs that *did* execute `SendInput` while the polled focus signal was inactive. It does **not** increment for downs blocked by the Phase 2 pre-down gate (those are labeled `blocked_unfocused` in `runtime_outcome`). | After the Phase 2 gate, the post-gate scenario is rare (the gate catches first; `_first_down_dispatched` stays false until first focused down). The counter remains useful for the residual window between last control-thread sample and SendInput completion — see §11 row 1. The mismatch with the original §0.1 wording ("only when attempted without active focus") is intentional: counting would-be attempts that got blocked is already covered by `abort_counts_by_reason["focus_lost"]` and `blocked_unfocused` outcomes; the counter is reserved for the rarer post-gate leak. | `dispatch_loop.py:467-470,573` |

---

## 12. Appendix A — Focus state machine (target)

```text
                    ┌─────────────┐
                    │  PLAYING    │
                    └──────┬──────┘
           require_focus   │
           & !sky_active   │  (poll or pre-down recheck)
                           ▼
              abort_input_safe(focus_lost)
              freeze elapsed (focus_pause_started)
                    ┌─────────────┐
                    │ FOCUS_PAUSE │
                    └──────┬──────┘
           sky_active again│
                           ▼
              grace (focus_restore_grace_us)
              release_all() again
              apply pause_time; clear focus_pause
                    ┌─────────────┐
                    │  PLAYING    │  (cursor continues)
                    └─────────────┘
```

Manual pause is parallel: `abort_input_safe(manual_pause)` → `MANUAL_PAUSE` → on resume only unfreeze (keys already up; no automatic re-hold of cancelled gens).

---

## 13. Appendix B — Mapping research findings → phases

| Research finding | Phase |
|------------------|-------|
| Focus lost cancel without release | 1 |
| Asymmetric abort paths | 1 |
| Focus TTL race before note-on | 2 |
| Partial chord silent drop | 3 |
| architecture.md wrong on focus release | 0 + 1 |
| Adaptive lead / completion-anchor keep | 4 (verify) |
| Cold-start lead=0 first samples | 4 |
| 1.0-frame probabilistic miss | 4 policy docs + 6 measure; no false “fix” |
| Watchdog already exists | no B1 reimplementation |
| UIPI elevation | 5 warn-only optional |

---

## 14. Appendix C — Command gate (every PR)

```powershell
uv run ruff check .
uv run pyright
uv run pytest
```

For backend/focus-only PRs, still run full pytest (lifecycle tests interact with engine).

---

*End of plan. Implementation starts at Phase 0; do not skip Phase 1 when “only optimizing timing” — stuck keys and unfocused note-ons dominate user-visible failures over sub-millisecond lead tweaks.*
