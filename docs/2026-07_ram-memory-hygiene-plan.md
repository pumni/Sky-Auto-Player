# RAM / Memory Hygiene Plan

Status: **active implementation plan** (2026-07-15)
Revision: **v2** — tinh chỉnh sau cross-reference với source code thực tế (2026-07-15)

Source audit: `ram_analysis_report.md` (Gemini Antigravity, 2026-07-15)
Verified against: `src/` as of 2026-07-15 (Python 3.14 freethreaded)

Owner role split:
- Implementer: coding agent / human
- Acceptance: green unit tests + optional `scripts/mem_after_play.py` before/after delta

This is **not** a scheduler/SendInput rewrite. It is a surgical hygiene pass: bound growth,
release holdouts, close OS handles, and harden free-threaded GC safety.

---

## 0. Review Summary (Audit vs Code)

### Verdict

The audit is **largely accurate** and useful. Severity labels need a few corrections so we do not
over-fix intentional design or overstate "leak" vs "peak during session".

| Audit ID | Audit severity | Code verdict | Adjusted priority | Notes |
|---|---|---|---|---|
| C1 dead `_peek_persistent_metadata` | CRITICAL | **Confirmed** | P0 | Lines 699–700 are unreachable duplicate; safe delete |
| H1 `telemetry.records` unbounded | HIGH | **Partial** | P1 | Cleared after successful `save()` (tested). **Peak during long telemetried play** still unbounded. Double-hold in `get_summary()` lines 664–673 also confirmed: iterates `self.records` a second time even though `rows` is already materialized |
| H2 `engine.actions` pinned | HIGH | **Intentional holdout** | P1 (evaluate) | Documented in engine.py:864 as source of truth for rebuild. `runtime_schedule` already nulled in finally (line 870). `actions` stays by design. Add explicit opt-in release API |
| H3 watchdog globals | HIGH | **Confirmed, low practical impact** | P2 | Process-lifetime; atexit only closes stdin |
| H4 `DryRunBackend.history` | HIGH | **Confirmed, test-only** | P2 | Production path uses `WinSendInputBackend` |
| H5 `_hook_proc_ref` not cleared | HIGH | **Confirmed** | P0 | `stop()` (line 135–138) never nulls ctypes callback → cycle risk under no-GIL |
| H6 metadata caches no RAM cap | HIGH | **Confirmed** | P1 | `_metadata_cache` / `_persistent_cache` grow with songs×sessions |
| M1 class-level telemetry dicts | MEDIUM | **Confirmed** | P2 | `TelemetryLogger.last_picker_cleanup` / `last_thread_census` — small; process-lifetime |
| M2 `command_event_handle` close | MEDIUM | **Confirmed** | P0 | `finally` at supervisor.py:463 gates on `not dispatch_thread.is_alive()` — handle leaked if thread stuck after join |
| M3 `_retired_resources` retained | MEDIUM | **Confirmed** | P2 | After `close_all`, lists keep closed resource refs |
| M4 hotkey queue unbounded | MEDIUM | **Confirmed** | P1 | Human-rate in practice; flood = DoS of RAM |
| M5 `gc.disable()` no fallback | MEDIUM | **Confirmed, high no-GIL risk** | P0 | `realtime.py:116` — no `__del__` / atexit safety net. Under Python 3.14 no-GIL, stuck-disabled GC means Textual widget reference cycles never reclaimed |
| M6 `_identity_cache` by `id(profile)` | MEDIUM | **Confirmed** | P2 | Cleared only via `repository.clear()` |
| M7 `DEBUG_LOG_BUFFER` unbounded | MEDIUM | **Confirmed** | P2 | Only when `PLAYBACK_DEBUG` env var set |
| M8 `AppConfig` mutate-in-place | MEDIUM | Design smell | P3 | Not a RAM leak; correctness only |
| M9 full `.clear()` eviction | MEDIUM | **Confirmed** | P1 | `_pkey_ram_cache` clears all at 2000; `_path_session_ram_cache` at 5000 — causes thundering-herd miss burst |
| M10 `normalized_index_map` LRU | MEDIUM | Soft | P2 | Already `lru_cache(2048)`; needs `cache_clear()` wired into library reload |
| M11–M14 UI lifecycle | MEDIUM | **Plausible, not yet reproduced** | P2-guard | Add a defensive guard in `on_unmount`; full rewrite deferred until objgraph repro |
| M15 `_ARRAY_CACHE` | MEDIUM | **Already fixed** | — | `engine.py` finally → `clear_array_cache()`; `test_post_play_memory_hygiene.py` |
| M16 `wake_errors` in options | MEDIUM | Small | P3 | Negligible (10 ints); dict spread churn is cosmetic |
| L\* items | LOW | Mostly accurate | P3 | Do when touching adjacent code |

### Already fixed (do not re-do)

| Item | Evidence |
|---|---|
| `_ARRAY_CACHE` cleared post-play | `engine.py` finally → `clear_array_cache()`; `test_post_play_memory_hygiene.py` |
| `records` cleared after successful save | `telemetry.save()` lines 766–767; same hygiene test |
| `status_by_generation` O(polyphony) | `test_runtime_dispatch_bounded_memory.py` |

### Risk model (what actually hurts users)

```
Peak RSS during long song + telemetry ON
  └─ H1 records list (dominant if enabled); double-hold in get_summary() makes peak 2×

RSS after many songs in one session (engine/UI kept alive)
  └─ H2 actions (per engine instance; one song's schedule)
  └─ H6 picker caches (grows with library × profile × FPS × tempo)
  └─ M9 thrash spikes CPU after cache nuke

Process stability / free-threaded correctness
  └─ M5 gc.disable() stuck ON  → unbounded cyclic garbage (Textual trees)
  └─ M2 Win32 handle leak if dispatch thread fails to die
  └─ H5 ctypes callback cycle pinning PlaybackControls

Test / dry-run only
  └─ H4 DryRunBackend.history
```

README claims ~100 MB RAM. Goal of this plan: keep **idle picker** and **post-play** reachable
holdouts flat across long multi-song sessions; cap **telemetry peak** when logging is on; never
leave **GC disabled** after a failed playback teardown.

### Resolved open questions (from v1)

**Q1: Telemetry truncation — is truncated in-memory CSV acceptable if full data is on disk?**

**Decision:** Yes. Preferred policy is **full disk + bounded RAM**:
- Open the CSV file once in `__init__` (when `enabled=True`) in append mode.
- Flush every `_TELEMETRY_FLUSH_CHUNK = 10_000` records, then clear the in-memory list.
- Running counters used by `get_summary()` are maintained separately so summary accuracy
  is never degraded. Summary stats (lateness, send durations) are computed from an
  **online Welford accumulator** rather than a full in-memory list after the cap.
- `_last_summary` cache remains the post-save get_summary() source. Tests using
  `retain_records_after_save=True` are unaffected (flush is still written to disk).

**Q2: `retain_actions` kwarg vs `release_song_data()` method?**

**Decision:** Use `PlaybackEngine.release_song_data()` method (not a `play()` kwarg).
- Avoids polluting the `play()` signature with an implementation-leak concern.
- Textual app calls it explicitly after `_log_timing_summary` returns.
- API callers who reuse an engine instance (tests, advanced callers) never call it
  unless they explicitly opt in.
- `release_song_data()` sets `self.actions = ()` (empty tuple, not `None`) so all
  `len(self.actions)` and iteration patterns remain safe without None-guards.
- Callers of `self.actions` in engine.py checked: only `total_time_us` computation
  (line 472, constructor) and `_execute_action`/`_process_wait_states` (compat path,
  not called after `play()` returns). Safe to empty after play.

**Q3: Picker max sizes — 1024/2048 vs library size?**

**Decision:** Use `_METADATA_CACHE_MAX = 2048` for `_metadata_cache`;
`_PERSISTENT_CACHE_MAX = 3000` for `_persistent_cache` (mirrors SQLite prune limit
at half of disk cap). `_pkey_ram_cache` max stays 2000; `_path_session_ram_cache` max
stays 5000 — but eviction changes from `.clear()` to LRU `popitem(last=False)`.
Rationale: 200 songs × 3 profiles × 5 FPS combos = 3000 entries max; 2048 covers
most real libraries with headroom.

---

## 1. Scope

### In scope

| Area | Paths |
|---|---|
| Picker caches | `src/sky_music/ui/picker_metadata.py`, `picker_theme.py` |
| Telemetry peak | `src/sky_music/orchestration/telemetry.py` |
| Engine holdouts | `src/sky_music/orchestration/engine.py` |
| RT / GC safety | `src/sky_music/infrastructure/realtime.py` |
| Hotkey hook | `src/sky_music/infrastructure/hotkey_hook.py` |
| Supervisor handles | `src/sky_music/orchestration/playback_supervisor.py` |
| Backend / dry-run / watchdog | `src/sky_music/infrastructure/backend.py` |
| Background scope hygiene | `src/sky_music/infrastructure/background.py` |
| Debug buffer | `src/main.py` |
| UI lifecycle guard | `src/sky_music/ui/textual_app/playback_app.py` (on_unmount only) |
| Tests | `tests/test_post_play_memory_hygiene.py` + new focused tests |
| Measure scripts | `scripts/mem_after_play.py` (baseline before any PR) |

### Out of scope

- Scheduler timing / hold formulas / SendInput hot path micro-opts
- Game focus strategy, anti-cheat, any process injection
- Broad Textual UI redesign or Textual widget refactor
- Rust migration (`docs/rust-migration-plan.md`)
- Freezing `AppConfig` (M8) unless a config PR is already open
- Claiming Task Manager RSS drops to zero (Windows sticky WS / pymalloc arenas —
  already documented in `engine.py:888`)
- M11–M14 full UI worker lifecycle rewrite (deferred until objgraph repro)

### Non-negotiables (from AGENTS.md)

- No game memory / injection / hooks beyond existing WH_KEYBOARD_LL control path
- SendInput only for note simulation
- Pure scheduler stays unit-testable; Windows code stays behind interfaces
- `uv run` for all Python; freethreaded interpreter

---

## 2. Design Principles

1. **Peak vs retain.** Bound *growth during play*; release *reachable objects after play*.
   Do not confuse Windows RSS plateaus (pymalloc arena stickiness) with Python leaks.
2. **Surgical.** One concern per PR; no drive-by refactors outside the touched file.
3. **Locks stay correct under no-GIL.** Every metadata cache mutation keeps `_cache_lock`
   / sibling locks. Do not invent lock-free caches.
4. **LRU over nuke.** Prefer `OrderedDict` + `popitem(last=False)` over full `.clear()`
   cliffs. Full clear discards hot entries along with cold ones, causing thundering-herd
   miss bursts.
5. **Always restore process-global state.** GC enable/disable, Win32 handles, ctypes
   callbacks must have `finally` / stop-path cleanup that runs even on exception.
6. **Tests prove reachable hygiene**, not RSS. Extend existing pattern in
   `test_post_play_memory_hygiene.py`.
7. **No-GIL first.** Under Python 3.14 freethreaded, ctypes function wrapper objects are
   not reliably tracked by the cycle detector. Any strong ref to a WINFUNCTYPE wrapper
   that is reachable from a Python object creates a potential permanent pinning. Clear
   all ctypes callbacks in their owner's `stop()` / `close()` path.

---

## 3. PR Plan (DAG)

Implement in this order. Later PRs may assume earlier ones are merged.

```text
PR-A (P0 safety / dead code) ──┐
PR-B (P1 telemetry peak)  ─────┼──► PR-D (picker LRU) ──► PR-E (lifecycle polish)
PR-C (P1 engine actions)  ─────┘
```

| PR | Title | Depends | Risk | Est. |
|---|---|---|---|---|
| **PR-A** | Safety + dead-code hygiene | — | Low | S |
| **PR-B** | Telemetry peak bound + summary double-hold fix | — | Medium | M |
| **PR-C** | Engine post-play `release_song_data()` | — | Low–Medium | S |
| **PR-D** | Picker cache LRU + library reload clear | PR-A (dead code gone) | Medium | M |
| **PR-E** | Backend / background / debug / UI guard polish | — | Low | S |

Parallelism: **A ∥ B ∥ C**, then **D**, then **E** (or fold E into A if tiny).

---

## 4. PR-A — Safety + Dead Code (P0)

### Goals

Eliminate confirmed bugs that can pin objects or leak OS resources regardless of song
length. All changes are low-risk and surgical.

### Changes

#### A1. Delete dead code — `picker_metadata.py`

In `_peek_persistent_metadata`, keep only the first `with _cache_lock` block:

```python
# BEFORE (lines 695–700):
with _cache_lock:
    return _persistent_cache.get(key)
with _cache_lock:                        # ← UNREACHABLE — delete this block
    return _persistent_cache.get(key)   # ← UNREACHABLE — delete this line

# AFTER:
with _cache_lock:
    return _persistent_cache.get(key)
```

#### A2. Clear ctypes callback — `hotkey_hook.py`

**Current `stop()` (lines 135–138):**
```python
def stop(self) -> None:
    if self._thread and self._thread.is_alive() and self._thread_id:
        user32.PostThreadMessageW(self._thread_id, WM_QUIT, 0, 0)
        self._thread.join(timeout=1.0)
```

**Required fix:** `_hook_proc_ref` must only be nulled **after the hook thread has
exited**. The WINFUNCTYPE wrapper must outlive the thread because `_run_pump`'s message
loop (line 125) calls back into `_hook_proc` via the OS. Nulling it before the thread
is confirmed dead would allow the OS to call through a freed wrapper under no-GIL
reference counting.

The correct null point is **inside `_run_pump`**, after `UnhookWindowsHookEx` returns
(line 129) — the message loop has exited and no further callbacks will fire:

```python
def _run_pump(self) -> None:
    self._thread_id = threading.get_native_id()
    self._hook_proc_ref = HOOKPROC(self._hook_proc)
    self._hook_id = user32.SetWindowsHookExW(WH_KEYBOARD_LL, self._hook_proc_ref, None, 0)

    if not self._hook_id:
        self._hook_proc_ref = None   # ← safe: hook was never installed
        return

    msg = wintypes.MSG()
    while user32.GetMessageW(ctypes.byref(msg), None, 0, 0) > 0:
        user32.TranslateMessage(ctypes.byref(msg))
        user32.DispatchMessageW(ctypes.byref(msg))

    user32.UnhookWindowsHookEx(self._hook_id)
    self._hook_id = None            # ← L5: prevent double-unhook
    self._hook_proc_ref = None      # ← safe: hook loop exited, no more callbacks
```

`stop()` itself does not touch `_hook_proc_ref` — it only signals the thread and joins:

```python
def stop(self) -> None:
    if self._thread and self._thread.is_alive() and self._thread_id:
        user32.PostThreadMessageW(self._thread_id, WM_QUIT, 0, 0)
        self._thread.join(timeout=2.0)   # increased from 1.0 for safety
    # _hook_proc_ref was already nulled inside _run_pump after UnhookWindowsHookEx.
    # _hook_id was already nulled there too.
    self._thread = None
```

**Also:** bound `event_queue` against keypress floods (M4):

```python
# __init__:
self.event_queue: queue.Queue[str] = queue.Queue(maxsize=64)

# In _hook_proc, replace self.event_queue.put(action) with:
try:
    self.event_queue.put_nowait(action)
except queue.Full:
    pass   # drop silently; hotkey flood is a user error, not a crash
# Still return 1 (swallow the key) so the OS chain is not disrupted.
```

#### A3. Unconditional handle close — `playback_supervisor.py`

**Current (lines 462–465):**
```python
finally:
    if not dispatch_thread.is_alive() and command_event_handle is not None:
        inputs.close_handle(command_event_handle)
        command_event_handle = None
```

The `is_alive()` guard means a handle is leaked whenever an exception escapes
the supervisor loop body before `dispatch_thread.join()` (line 461) completes.
After `join()`, the thread is always dead — the condition is vacuously True on the
happy path. Remove the guard:

```python
finally:
    if command_event_handle is not None:
        with contextlib.suppress(Exception):
            inputs.close_handle(command_event_handle)
        command_event_handle = None
```

`contextlib.suppress(Exception)` prevents a close failure from masking the original
exception (already the convention for cleanup in this codebase — see `engine.py:877`).

#### A4. GC restore fallback — `realtime.py`

`RealtimeProcessScope.__enter__` calls `gc.disable()` with no `__del__` fallback.
Under Python 3.14 no-GIL, an abandoned scope leaves cyclic GC permanently disabled,
causing Textual widget trees (which have parent/child reference cycles) to never be
reclaimed.

**Cross-reference with source:** `__slots__` is already defined (line 91–96), so
`__del__` is supported. Add `_restore()` and wire it into both `__exit__` and `__del__`:

```python
def _restore(self) -> None:
    """Re-enable GC and restore switch interval. Idempotent."""
    if self._gc_was_enabled:
        with contextlib.suppress(Exception):
            gc.enable()
        self._gc_was_enabled = False
    if self._old_switch_interval is not None:
        with contextlib.suppress(Exception):
            sys.setswitchinterval(self._old_switch_interval)
        self._old_switch_interval = None

def __exit__(
    self,
    exc_type: type[BaseException] | None,
    exc: BaseException | None,
    tb: TracebackType | None,
) -> None:
    self._restore()

def __del__(self) -> None:
    # Best-effort fallback if __exit__ was never called (scope abandoned without
    # with-statement). suppress all exceptions — __del__ must not raise.
    with contextlib.suppress(Exception):
        self._restore()
```

**Note:** Do not add `atexit.register` on top of `__del__`. `__del__` is sufficient
for abandoned scopes in both GIL and no-GIL CPython, because `_gc_was_enabled` tracks
the conditional state correctly (the scope only re-enables GC if it was the one that
disabled it, line 113).

### Tests

| Test | Assert |
|---|---|
| `test_peek_persistent_metadata_single_lock_path` | Grep/ast: exactly one `with _cache_lock` in `_peek_persistent_metadata` body |
| `test_hotkey_hook_stop_clears_proc_ref` | After `stop()` returns and thread joined, `hook._hook_proc_ref is None` and `hook._hook_id is None` |
| `test_hotkey_queue_drops_on_full` | Fill queue to maxsize, one more `_hook_proc` call → no `queue.Full` raised, queue size stays at maxsize |
| `test_realtime_scope_restores_gc_on_del` | Enter scope (GC disabled), `del scope` without calling `__exit__` → `gc.isenabled()` is True |
| `test_realtime_scope_del_idempotent_after_exit` | `__exit__` then `del` → no double-enable error, `gc.isenabled()` True |
| Supervisor unit / existing playback tests | Handle close does not raise on normal join; no regression |

### Acceptance

- `uv run ruff check . && uv run pyright && uv run pytest` green
- No change to SendInput timing paths

---

## 5. PR-B — Telemetry Peak Bound (P1)

### Goals

Stop multi-hundred-MB peaks when telemetry is enabled on long/dense songs, without
breaking summary accuracy, CSV completeness, or existing test hooks.

### Problem restatement (confirmed against source)

1. `self.records` is a plain `list[]` (line 182) appended on every dispatch event
   (line 274). Never cleared during playback.
2. `get_summary()` at line 360 materializes `rows = [r._materialize() for r in self.records]`
   — a full parallel dict list — while `self.records` is still alive. Lines 664–673
   then iterate `self.records` **again** (not `rows`) for `deferred_release_count` and
   `release_deferral_us`, holding both simultaneously at peak.
3. Post-save clear (line 767) already works and is tested. The fix targets the
   **during-play peak** and the **double-hold at summary time**.

### Design (resolved — see Q1 in §0)

Selected: **incremental CSV flush + online summary accumulators** (B2+B3 hybrid).

```
_TELEMETRY_FLUSH_CHUNK = 10_000   # flush after this many records; tunable constant
```

#### B1. Fix double-hold in `get_summary()` immediately (zero-risk quick win)

Replace the two `self.records` iterations at lines 664–673 with `rows`-based
equivalents. `rows` is already fully materialized dicts on line 360:

```python
# BEFORE (lines 664–673) — iterates self.records a second time:
"deferred_release_count": sum(
    1 for record in self.records if int(record.get("deferred_by_us", 0)) > 0
),
"release_deferral_us": _stats(
    [int(record.get("deferred_by_us", 0))
     for record in self.records
     if int(record.get("deferred_by_us", 0)) > 0]
),

# AFTER — use rows (already materialized above line 360):
"deferred_release_count": sum(
    1 for r in rows if int(r.get("deferred_by_us", 0)) > 0
),
"release_deferral_us": _stats(
    [int(r.get("deferred_by_us", 0))
     for r in rows if int(r.get("deferred_by_us", 0)) > 0]
),
```

This alone halves the peak RSS at summary time.

#### B2. Incremental CSV flush during playback

Open the CSV file once in `__init__` (when `enabled=True`), write header, then flush
every `_TELEMETRY_FLUSH_CHUNK` records inside `record()`:

```python
# __init__ (when self.enabled):
self._csv_writer: csv.DictWriter | None = None
self._csv_file: IO | None = None
if self.enabled:
    logs_dir = Path("logs")
    logs_dir.mkdir(parents=True, exist_ok=True)
    self.log_filepath = logs_dir / f"playback_telemetry_{self.run_id}.csv"
    self._csv_file = self.log_filepath.open("w", newline="", encoding="utf-8")
    self._csv_writer = csv.DictWriter(self._csv_file, fieldnames=_CSV_FIELDS)
    self._csv_writer.writeheader()

# record() — after append:
self.records.append(TelemetryRecord(...))
if len(self.records) >= _TELEMETRY_FLUSH_CHUNK:
    self._flush_records_to_csv()

def _flush_records_to_csv(self) -> None:
    """Write accumulated records to the open CSV and clear the in-memory list."""
    if self._csv_writer is None or not self.records:
        return
    with contextlib.suppress(Exception):
        self._csv_writer.writerows(self.records)
        self._csv_file.flush()          # type: ignore[union-attr]
    self.records = []

# save() — simplified, no longer re-opens the file:
def save(self) -> None:
    if not self.enabled or not self.log_filepath:
        return
    try:
        self._flush_records_to_csv()
        if self._csv_file is not None:
            self._csv_file.close()
            self._csv_file = None
            self._csv_writer = None
        summary = self.get_summary()
        if summary is None:
            return
        summary["timestamp"] = time.strftime('%Y-%m-%d %H:%M:%S')
        summary_path = self.log_filepath.with_suffix(".summary.json")
        with summary_path.open("w", encoding="utf-8") as f:
            json.dump(summary, f, indent=2)
    except Exception as e:
        sys.stderr.write(f"[telemetry] failed to save metrics: {e}\n")
        return
    if not self._retain_records_after_save:
        self.records = []
```

**Backward compatibility:** `retain_records_after_save=True` still works — flush is
still written to disk, but in-memory records are preserved after `save()`.

**Summary accuracy:** `get_summary()` already works from `self.records` (the tail
since last flush). After flushing, `self.records` is empty; `get_summary()` returns
`self._last_summary` (the cache). This is correct for post-play callers.
During-play callers of `get_summary()` (if any) see only the unflushed tail —
this is acceptable since the summary is a post-play artifact.

### Engine interaction

- Do **not** break `retain_records_after_save` test hook.
- `test_post_play_memory_hygiene.py` must still pass (empty records after save).
- CSV is now written incrementally — tests that read the CSV after play still pass
  since the file is flushed and closed in `save()`.

### New tests

| Test | Assert |
|---|---|
| `test_telemetry_summary_uses_rows_not_second_records_pass` | `get_summary()["deferred_release_count"]` matches manual count; no duplicate iteration |
| `test_telemetry_flush_chunk_clears_records` | After `_TELEMETRY_FLUSH_CHUNK + 1` `record()` calls, `len(telemetry.records) == 1` |
| `test_telemetry_csv_written_incrementally` | After flush chunk, CSV file exists and has `_TELEMETRY_FLUSH_CHUNK` data rows |
| `test_telemetry_save_closes_csv_file` | After `save()`, `_csv_file is None`; CSV + JSON both readable |
| Existing hygiene tests | Unchanged |

### Acceptance

- Peak reachable `TelemetryRecord` count bounded at `_TELEMETRY_FLUSH_CHUNK` under
  synthetic 200k-event stress
- Summary still has non-null `_last_summary` after save
- CSV on disk is complete (all events present — flush is additive append)

---

## 6. PR-C — Engine `release_song_data()` (P1)

### Goals

Clarify and implement opt-in shrink of post-play `self.actions` retain.

### Current contract (do not break blindly)

From `engine.py:864`:

> `self.actions` is the **persistent source of truth** for rebuilding `runtime_schedule`
> on a second `play()` of the same engine.

- `runtime_schedule` is already nulled in `finally` (line 870). ✅
- Textual UI creates a **new** `PlaybackEngine` per play — no reuse in production.
- Multi-play reuse of one engine is tests / advanced callers only.
- `self.actions = ()` (empty tuple) is safe: all callers use `for action in self.actions`
  or `max(... for action in actions, default=0)` — both safe on empty tuple.

### Implementation

Add `release_song_data()` method to `PlaybackEngine`:

```python
def release_song_data(self) -> None:
    """Drop per-song schedule data after play() has returned.

    Call this only when the engine will not be reused for a second play() on the
    same song. The Textual app calls it after _log_timing_summary returns.
    After this call, self.actions == () and runtime_schedule is None.
    A subsequent play() call will raise (actions is empty), which is the correct
    signal that the engine needs to be rebuilt.
    """
    self.actions = ()
    self.runtime_schedule = None
    self._runtime_coordinator = None
```

**Call site in Textual app** (`playback_app.py` or `app.py`, after `_log_timing_summary`):

```python
# After engine.play() returns and _log_timing_summary is done:
with contextlib.suppress(Exception):
    self.engine.release_song_data()
```

Do **not** call it inside `play()` itself — that would break any caller that reuses
the engine, and it would fire before `_log_timing_summary` which reads `self.actions`
indirectly via telemetry.

### Tests

- Existing engine tests default to not calling `release_song_data()` → second `play()`
  still works if any test reuses engine.
- New: `test_release_song_data_empties_actions` — after `release_song_data()`,
  `engine.actions == ()` and `engine.runtime_schedule is None`.
- New: `scripts/mem_after_play.py` baseline delta shows smaller post-play RSS when
  Textual path calls `release_song_data()`.

### Acceptance

- No regression in Textual playback
- `engine.actions == ()` measurable after UI song completion

---

## 7. PR-D — Picker Cache LRU (P1)

### Goals

Bound picker RAM; stop full-clear thrashing on `_pkey_ram_cache` and
`_path_session_ram_cache`; cap `_metadata_cache` and `_persistent_cache`;
wire theme LRU invalidation into library reload.

### Targets

| Cache | Today | Target |
|---|---|---|
| `_metadata_cache` | unbounded dict | `OrderedDict`, max 2048 entries, LRU on get/set |
| `_persistent_cache` | unbounded dict (SQLite has 6000 disk rows) | `OrderedDict`, max 3000, LRU, mirrors disk prune at ~50% |
| `_pkey_ram_cache` | `.clear()` at 2000 | LRU max 2000 (no full clear) |
| `_path_session_ram_cache` | `.clear()` at 5000 | LRU max 5000 (no full clear) |
| `normalized_index_map` | `lru_cache(2048)` | call `cache_clear()` from `clear_metadata_cache()` |

### Implementation

**LRU helpers** (add as module-level private helpers in `picker_metadata.py`):

```python
from collections import OrderedDict

def _lru_set(cache: OrderedDict, key: object, value: object, *, maxsize: int) -> None:
    """Insert or update key with LRU eviction. All access under caller's lock."""
    if key in cache:
        cache.move_to_end(key)
    cache[key] = value
    while len(cache) > maxsize:
        cache.popitem(last=False)   # evict least-recently-used (oldest) entry

def _lru_get(cache: OrderedDict, key: object) -> object:
    """Return value for key and promote to MRU position; return None if absent."""
    # IMPORTANT: check key membership, not truthiness of value.
    # A valid SongUiMetadata may be falsy in future; id-based check is correct.
    if key not in cache:
        return None
    cache.move_to_end(key)
    return cache[key]
```

> **Bug fix from v1:** The original plan used `if value is not None:` which would
> silently skip the LRU promotion for a legitimately-stored `None` value (or any
> falsy `SongUiMetadata`). The correct check is `if key not in cache: return None`.

**Convert existing dicts to `OrderedDict`** and replace all direct dict access with
`_lru_set` / `_lru_get` under the existing per-cache locks.

**Replace full-clear with LRU eviction** in all four caches:

```python
# BEFORE (e.g. _pkey_ram_cache):
if len(_pkey_ram_cache) > 2000:
    _pkey_ram_cache.clear()

# AFTER: remove the size check entirely — _lru_set handles eviction inline.
# Just replace: _pkey_ram_cache[key] = value
# With:         _lru_set(_pkey_ram_cache, key, value, maxsize=_PKEY_RAM_CACHE_MAX)
```

**Wire theme cache into library invalidation** inside `clear_metadata_cache()`:

```python
# Inside clear_metadata_cache() — lazy import to avoid circular:
def clear_metadata_cache(*, clear_persistent: bool = False) -> None:
    ...  # existing body
    # Wire LRU invalidation for theme search index
    try:
        from sky_music.ui.picker_theme import normalized_index_map
        normalized_index_map.cache_clear()
    except Exception:
        pass   # best-effort; never raise from a cache-clear utility
```

### Tests

| Test | Assert |
|---|---|
| `test_metadata_cache_lru_evicts_oldest` | Insert `_METADATA_CACHE_MAX + 1` distinct keys → oldest key absent, newest present |
| `test_metadata_cache_lru_promotes_on_get` | Get oldest key → it survives the next eviction; a never-accessed key is evicted first |
| `test_pkey_ram_cache_no_full_clear_cliff` | Fill past old 2000 threshold; most-recently-accessed key still present |
| `test_persistent_cache_bounded` | Insert `_PERSISTENT_CACHE_MAX + 10` → `len(_persistent_cache) == _PERSISTENT_CACHE_MAX` |
| `test_clear_metadata_cache_clears_theme_lru` | After `clear_metadata_cache()`, `normalized_index_map.cache_info().currsize == 0` |
| Existing picker metadata tests | All pass unchanged |

### Acceptance

- Long picker session with 200+ songs × profile switches stays under configured
  entry caps
- No render-path lock regression (hold locks only for dict ops, never for I/O)

---

## 8. PR-E — Lifecycle Polish (P2)

Small fixes, batchable or folded into PR-A if tiny enough.

| ID | File | Change |
|---|---|---|
| H3 | `backend.py` | In atexit handler: `_watchdog_proc.stdin.close(); _watchdog_proc.wait(timeout=2); _watchdog_proc = None; _watchdog_thread = None` |
| H4 | `backend.py` | `DryRunBackend.history = deque(maxlen=10_000)` (replaces plain list) |
| M1 | `telemetry.py` | Reset `TelemetryLogger.last_picker_cleanup = None` and `last_thread_census = None` at start of each `play()` / logger init |
| M3 | `background.py` | After successful `close_all()`, `self._resources.clear(); self._retired_resources.clear()` |
| M7 | `main.py` | In `debug_log()`: if `len(DEBUG_LOG_BUFFER) >= 500`, call `flush_debug_log()` before append |
| M11-guard | `playback_app.py` | In `on_unmount`: if `self.engine is not None`, log warning + call `_safe_finish()` (best-effort); document as guard not guarantee |
| L2 | `runtime_session.py` | After playback end, `RUNTIME_STATE._state.session = None` (or expose `clear_session()` on proxy) |
| L8 | `doctor.py` | Module-level `_winmm: ctypes.WinDLL | None = None`; lazy init on first `check_timer_resolution()` call; reuse thereafter |
| M6 | `song_repository.py` | Document identity cache; add `clear()` call in `SongRepository.reload()`; consider `weakref.WeakValueDictionary` only if profiles are confirmed long-lived objects |

### M11–M14 guard rationale

Full rewrite of `SnapshotRenderer` / `MetadataCoordinator` lifecycle is deferred
(requires `objgraph` repro). The minimal guard added here:

```python
# playback_app.py — on_unmount:
def on_unmount(self) -> None:
    if self.engine is not None:
        # Engine still alive at unmount — this is unexpected.
        # _safe_finish() is idempotent; call it defensively.
        self.post_message(PlaybackApp.PlaybackFinished())  # or call directly
        self.log.warning("[memory] engine alive at on_unmount — calling _safe_finish()")
        with contextlib.suppress(Exception):
            self._safe_finish(result=None, error=None)
```

This ensures the engine reference is cleared even if the Textual lifecycle
skip `_safe_finish`. It is not a substitute for a full repro-driven fix.

### Tests

- `test_dryrun_history_bounded` — `DryRunBackend.history` capped at `maxlen=10_000`
- `test_background_scope_close_all_clears_lists` — after `close_all()`,
  `len(scope._resources) == 0 and len(scope._retired_resources) == 0`
- `test_debug_log_buffer_auto_flush` — after 501 `debug_log()` calls,
  buffer length resets to `<= 1`

---

## 9. Measurement & Verification Protocol

### Baseline (before any PR)

```powershell
uv run python scripts/mem_after_play.py
```

Record and commit the baseline numbers. This script must exist before PR-B/C work
begins. Measure:

- `len(engine.telemetry.records)` after play (pre-PR-B: full event list; post: 0)
- `sys.getsizeof(engine.actions)` after play (pre-PR-C: song tuple; post: `()`)
- `len(_metadata_cache)` after picker session (pre-PR-D: unbounded; post: ≤ 2048)
- Process RSS from `psutil.Process().memory_info().rss` at idle picker

### Automated (required every PR)

```powershell
uv run ruff check .
uv run pyright
uv run pytest
```

Narrow suites while iterating:

```powershell
uv run pytest tests/test_post_play_memory_hygiene.py tests/test_runtime_dispatch_bounded_memory.py -q
```

### Reachable-object probes (recommended before/after PR-B/C)

```powershell
uv run python scripts/mem_after_play.py
```

Record:

- `len(engine.telemetry.records)` after play
- `sys.getsizeof` / tracemalloc top for `actions`, caches
- Not raw Task Manager RSS alone (pymalloc arena stickiness makes RSS an unreliable
  metric for short-term changes)

### Manual free-threaded smoke

1. Launch Textual UI, open picker with full `songs/` library, scroll + search 2 minutes
   → RSS should plateau (PR-D).
2. Play one long dense song with telemetry on → peak bounded, no OOM (PR-B).
3. Kill playback mid-song / raise artificial exception path → `gc.isenabled()` True
   after return to picker (PR-A A4).
4. Spam hotkeys → queue does not grow without bound (PR-A A2).
5. Close Textual window while engine is playing → no warning log about engine alive
   at unmount (PR-E guard; or warning logged but engine cleaned up).

### Success criteria (project-level)

| Metric | Target |
|---|---|
| Post-play `telemetry.records` | empty (already) |
| During-play records | ≤ `_TELEMETRY_FLUSH_CHUNK` (10 000) at any point |
| `_metadata_cache` entries | ≤ 2048 |
| `_persistent_cache` entries | ≤ 3000 |
| GC after RT scope | enabled if was enabled before |
| `_hook_proc_ref` after hook thread exit | `None` |
| `_hook_id` after hook thread exit | `None` |
| `command_event_handle` | closed on all supervisor exit paths |
| `engine.actions` post-play (Textual path) | `()` after `release_song_data()` |
| README "~100mb" | remains directionally true for normal use without telemetried 30+ min stress |

---

## 10. Implementation Checklist (per PR)

- [ ] Branch from latest main
- [ ] Run `scripts/mem_after_play.py` baseline on this branch before starting
- [ ] Touch only files listed for that PR
- [ ] Type hints on new helpers
- [ ] No new dependencies
- [ ] Tests first or with fix (fail → green)
- [ ] `uv run ruff check . && uv run pyright && uv run pytest`
- [ ] Short CHANGELOG note under Fixed/Changed
- [ ] Do not expand PyInstaller `excludes` without grepping `src/`

---

## 11. Suggested Commit Sequence

```text
1. fix(picker): remove dead duplicate lock block in _peek_persistent_metadata
2. fix(hotkey): null ctypes callback inside _run_pump after UnhookWindowsHookEx; bound queue
3. fix(supervisor): unconditionally close command_event_handle in finally
4. fix(realtime): add __del__ fallback to restore gc if RealtimeProcessScope abandoned
5. fix(telemetry): fix double-materialize in get_summary; add incremental CSV flush
6. feat(engine): add release_song_data() for opt-in post-play actions release
7. fix(picker): LRU-bound all four metadata caches; wire theme lru_cache into clear
8. chore: watchdog/background/debug/ui-guard hygiene leftovers
```

---

## 12. Mapping Audit → PR

| Audit | PR |
|---|---|
| C1 dead code | A |
| H5, M2, M4, M5 | A |
| H1 + summary double-hold | B |
| H2 | C |
| H6, M9, M10 | D |
| H3, H4, M1, M3, M7, M11-guard, L2, L8, M6 | E |
| M15, records post-save, status_by_generation | Done — no PR |
| M8, M16, most L\* | Backlog / adjacent |

---

## 13. What Not To Do

- Do not re-enable cyclic GC *during* RT dispatch (defeats the jitter-reduction purpose
  of the pause).
- Do not clear `_ARRAY_CACHE` mid-dispatch (already cleared post-join only, per
  `test_post_play_memory_hygiene.py` contract).
- Do not replace locks with "faster" unlocked module dicts under freethreaded — data
  races on shared dicts are undefined behaviour under no-GIL.
- Do not use full-process `gc.collect()` loops in hot UI render paths.
- Do not treat Windows Working Set stickiness as a Python bug after reachable objects
  are released (already documented in `engine.py:888`).
- Do not null `_hook_proc_ref` from `stop()` (caller thread) while the hook thread
  may still be executing callbacks — null it only from inside `_run_pump` after
  `UnhookWindowsHookEx` returns.
- Do not add a `retain_actions: bool` kwarg to `play()` — use `release_song_data()`
  as a separate explicit post-play call instead.

---

## 14. Exit Criteria for the Whole Initiative

All of the following:

1. PR-A through PR-D merged (PR-E optional polish, recommended).
2. Full test gate green on freethreaded 3.14:
   `uv run ruff check . && uv run pyright && uv run pytest`
3. `test_post_play_memory_hygiene` + new cap/LRU/GC/hook tests green.
4. `scripts/mem_after_play.py` post-PR numbers recorded and committed in
   `docs/mem_baseline_after_hygiene.txt`.
5. Manual smoke: multi-song session without monotonic unbounded Python heap growth
   attributable to picker caches or telemetry records.
6. This plan stamped **Completed** and moved to `docs/archive/` with outcome notes
   (peak numbers, final cap constants, any deferred items).
