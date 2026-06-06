"""Standalone SendInput latency benchmark — isolates the OS send path from the game.

Measures how long each `SendInput` call actually takes on THIS machine RIGHT NOW, using the
project's real send path (send_scan_code_batch). It types harmless characters into whatever window
is focused, so:

    FOCUS AN EMPTY NOTEPAD before running. Do NOT focus the game or anything important.

Run:  uv run python tests/bench_sendinput.py
Then compare the numbers against the in-game telemetry send_duration:
  - If p50 here is ~30-90us  -> the OS send path is healthy; the in-game elevation is GAME-specific
    (game input thread / anti-cheat on the game window).
  - If p50 here is ~250us+   -> a SYSTEM-WIDE low-level keyboard hook (Discord/OBS/GeForce/RGB/AHK/
    recorder/anti-cheat) is intercepting every injected event. Close those apps and re-run.
"""
from __future__ import annotations

import sys
import time

# 'A' scancode (set 1) = 0x1E. Harmless: types 'a' into the focused Notepad.
SCAN_A = 0x1E
ITERATIONS = 2000


def main() -> int:
    try:
        sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    except Exception:
        pass
    from sky_music.platform.win32.inputs import (
        send_scan_code_batch,
        enable_high_precision_timers,
        disable_high_precision_timers,
    )

    print("Focus an EMPTY NOTEPAD now. Starting in 3 seconds...")
    time.sleep(3.0)
    enable_high_precision_timers()
    downs: list[int] = []
    ups: list[int] = []
    try:
        for _ in range(ITERATIONS):
            t0 = time.perf_counter_ns()
            send_scan_code_batch((SCAN_A,), key_up=False)
            t1 = time.perf_counter_ns()
            send_scan_code_batch((SCAN_A,), key_up=True)
            t2 = time.perf_counter_ns()
            downs.append((t1 - t0) // 1000)
            ups.append((t2 - t1) // 1000)
            time.sleep(0.001)  # don't flood; ~1ms spacing like real playback gaps
    finally:
        disable_high_precision_timers()

    def stats(xs: list[int], label: str) -> None:
        xs = sorted(xs)
        n = len(xs)
        p = lambda q: xs[min(n - 1, int(q * n))]
        print(
            f"  {label}: n={n} p50={p(.5)} p90={p(.9)} p95={p(.95)} p99={p(.99)} "
            f"max={xs[-1]} (us)"
        )

    print(f"\nSendInput latency over {ITERATIONS} iterations:")
    stats(downs, "key_down")
    stats(ups, "key_up  ")
    print(
        "\nCompare p50 to in-game telemetry send_duration_us.\n"
        "  ~30-90us here  -> send path healthy; investigate the game window specifically.\n"
        "  ~250us+ here   -> external system-wide input hook; close overlay/recorder/RGB/AHK apps."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
