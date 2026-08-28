# Sky Auto Player — Tuning Presets

This document maps environment types to CLI flags for Sky Auto Player.
**All flags listed here already exist in the codebase** (`src/main.py` argparse).
No code changes are needed — simply pass the flags that match your environment.

Run `uv run python src/main.py --doctor` first to confirm your interpreter is the
free-threaded 3.14 build (the runtime fail-fast at `main()` startup enforces this)
and to inspect MMCSS status and key-mapping health before choosing a preset.

---

## Presets

| Preset | When to use | Command / Notes |
|---|---|---|
| **Free-threaded (`python3.14t`, no GIL) — the only supported build** | All users — the runtime enforces this at startup | *(no extra flags — switch-interval tuning auto-skips when GIL is absent, as of v2.2.2)* |
| **Weak machine / MMCSS unavailable** | `rt_priority_acquired: off` appears in telemetry (`--inspect-telemetry`) | `--rt-priority-mode highest` |
| **Jitter investigation** | Diagnosing timing nicks or frame drops | `--no-event-wait` then compare telemetry `p99_lateness_us` before/after |
| **Maximum compatibility** | Unusual sleep/timer behaviour, VM, or sandboxed environment | `--no-waitable-timer --no-event-wait` |
| **Dispatch on legacy single thread** | Debugging threading interactions | `--single-thread-dispatch` |
| **GC pause disabled** | Profiling GC contribution to jitter | `--no-gc-pause` (compare `send_duration_us` distribution) |
| **Timer guard disabled** | Investigating 1 ms timer-resolution failures | `--no-timer-guard` (debug only) |

---

## How to verify a preset is working

After launching playback, press `t` (or `F3` if in TUI mode) to open the telemetry overlay, or
run `--inspect-telemetry` after a session ends.

Key fields to check:

| Field | Healthy value | What it means |
|---|---|---|
| `sender_clean` | `true` | No send failures |
| `rt_priority_acquired` | `on` | MMCSS / thread priority ladder active |
| `switch_interval_tuning` | matches your flag | Flag wired correctly |
| `gil_enabled` | `false` | Confirms the running interpreter is the supported free-threaded 3.14 build (any other value is now rejected at startup — see [Notes for free-threaded forkers](#notes-for-free-threaded-forkers-python314t) below) |
| `p99_lateness_us` | < 2 000 µs | Dispatch is keeping up with schedule |
| `drop_count` | `0` | No events dropped |

---

## UPX / strip note

The release binary has `strip=False` and `upx=False` (see `Sky-Player.spec`).
UPX can trigger false-positive antivirus alerts on Windows — this is intentional.
Forkers who want a smaller binary can enable UPX manually in the spec and accept the
AV risk themselves.

---

## Notes for free-threaded forkers (`python3.14t`)

Sky Auto Player is pinned to the free-threaded CPython 3.14 build by the
**interpreter pair invariant** — `.python-version` (`3.14.7+freethreaded`) and
`pyproject.toml`'s `requires-python` (`>=3.14,<3.15`) move together. The
project deliberately makes the no-GIL runtime a hard requirement (the dispatch
loop and the Textual UI thread must not contend on the GIL).
The `--no-switch-interval-tuning` flag is therefore a no-op in practice — the
runtime skips `setswitchinterval` automatically when `sys._is_gil_enabled()`
returns `False`.

### Runtime fail-fast guard

`main()` calls `assert_free_threaded_runtime()` (defined in
`infrastructure/realtime.py`) before any selftests or playback wiring. The
guard raises `FreeThreadedRuntimeError` if **either** of the following fails:

* the **build** is not free-threaded (`sysconfig.get_config_var("Py_GIL_DISABLED")`
  is missing or not equal to `1`); **or**
* the **runtime** has the GIL enabled (`sys._is_gil_enabled()` returns `True`,
  or the probe is missing on an interpreter older than 3.13).

`main()` prints a banner that names the failed condition and exits with code 2
(via `_wait_key_and_exit(2)`). The app never proceeds into the dispatch loop
on a non-free-threaded interpreter — silently installing a stock 3.14 build
under the app would otherwise deadlock the UI thread against the dispatch
spin.

### Building a `python3.14t` binary

To build a `python3.14t` binary, use a separate PyInstaller invocation against
a `3.14t` interpreter. This is out of scope for the default release pipeline
and is left as a fork exercise.

---

## `requires-python` policy

`pyproject.toml` pins `requires-python = ">=3.14,<3.15"`, paired with
`.python-version = "3.14.7+freethreaded"` (the interpreter-pair invariant
documented in `AGENTS.md` and enforced at runtime by
`assert_free_threaded_runtime()` — see above). The free-threaded build of
3.14 is mandatory: the dispatch loop and the Textual UI thread must not
contend on the GIL. The `getattr(sys, "_is_gil_enabled", None)` probe in
`realtime.py` is used only to detect whether the runtime GIL is still active
in a build that *advertises* free-threadedness (e.g. a forker who flipped
`PYTHON_GIL=1` on a `3.14t` build); it is not a backward-compat path for
older Python versions — those are rejected at startup.
