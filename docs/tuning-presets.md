# Sky Player — Tuning Presets

This document maps environment types to CLI flags for Sky Player.
**All flags listed here already exist in the codebase** (`src/main.py` argparse).
No code changes are needed — simply pass the flags that match your environment.

Run `uv run python src/main.py --doctor` first to see your build's GIL state, Python
version, MMCSS status, and key-mapping health before choosing a preset.

---

## Presets

| Preset | When to use | Command / Notes |
|---|---|---|
| **Default (stock CPython 3.14, GIL enabled)** | Most users — standard CPython release | *(no extra flags — defaults are already optimised)* |
| **Free-threaded (`python3.14t`, no GIL)** | Forkers using a no-GIL build | *(no extra flags — switch-interval tuning auto-skips when GIL is absent, as of v2.2.2)* |
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
| `gil_enabled` | `true` (stock) / `false` (3.14t) | Confirms which build you're on |
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

Sky Player works on free-threaded CPython without any flag changes.
The `--no-switch-interval-tuning` flag is a no-op in this case (the runtime skips
`setswitchinterval` automatically when `sys._is_gil_enabled()` returns `False`).

To build a `python3.14t` binary, use a separate PyInstaller invocation against a
`3.14t` interpreter. This is out of scope for the default release pipeline and is left
as a fork exercise.

---

## `requires-python` policy

`pyproject.toml` pins `requires-python = ">=3.11,<3.15"` to keep the window open for
forkers on older (but supported) Python versions. The `getattr(sys, "_is_gil_enabled", None)`
probe in `realtime.py` handles backward compatibility gracefully — no minimum version
bump is needed for the GIL-awareness feature.
