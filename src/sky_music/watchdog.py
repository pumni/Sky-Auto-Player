"""
Watchdog subprocess to ensure keys are released if the main process crashes or stalls.

Stuck-key safety contract (review of main@7c548527 §2):
    * EOF (parent pipe closed) — parent process died or exited. We treat this as a crash and
      immediately release all 15 Sky keys, idempotently with the parent's own atexit cleanup.
      Belt-and-braces: if the parent's release_all already ran, the extra KEYUP is a no-op.
    * Stall (parent alive but no heartbeat bytes for an extended window) — we require
      multiple consecutive poll discoveries of an aged heartbeat before panicking, so a brief
      scheduling hiccup or one late heartbeat flush (the legacy 250 ms slack) cannot fire a
      full-15 KEYUP while the dispatch thread is still mid-note.

Telemetry is emitted to ``stderr`` on panic with ``panic_reason`` (``eof`` / ``stall`` /
``read_error``) and the heartbeat age at the panic instant. The parent captures stderr via
``subprocess.DEVNULL`` by default, so this is forensic-only — it surfaces only when the
parent routes watchdog stderr somewhere observable.
"""
import contextlib
import sys
import threading
import time

from sky_music.layouts import SKY_15_SCAN_CODES
from sky_music.platform.win32.inputs import send_scan_code_batch

# Heartbeat cadence the parent promises (``backend.py`` heartbeat thread sleeps
# ``HEARTBEAT_INTERVAL_S`` between writes). Documented here so the stall threshold can be
# expressed in heartbeat counts rather than opaque absolute seconds.
HEARTBEAT_INTERVAL_S = 0.5

# Stall detection: age > STALL_AFTER_S AND ticked past STALL_TICK_THRESHOLD consecutive poll
# observations of that age. ``STALL_AFTER_S = 3 × heartbeat`` means a single late heartbeat
# flush (the dominant jitter source under load) cannot by itself reach the threshold — we
# need three full heartbeats to be absent before we even start counting consecutive ticks.
STALL_AFTER_S = HEARTBEAT_INTERVAL_S * 3  # 1.5s

# Number of 50ms poll ticks at-or-above STALL_AFTER_S required to confirm a real stall.
# With STALL_AFTER_S = 1.5s and STALL_TICK_THRESHOLD = 4 (4 × 50ms = 200ms), the effective
# minimum heartbeat age at panic is ~1.7s — 3 missing heartbeats plus a tolerance band to
# absorb the reader thread's own scheduling delay. The previous 0.75s single-shot stall
# threshold had only ~250 ms of slack before firing; this restores real-world resilience
# while keeping panic latency bounded for true stalls.
STALL_TICK_THRESHOLD = 4
POLL_INTERVAL_S = 0.05


def panic_release_all() -> None:
    """Send KEYUP for all 15 Sky scan codes. Idempotent and best-effort."""
    with contextlib.suppress(Exception):
        send_scan_code_batch(SKY_15_SCAN_CODES, key_up=True)


def main() -> None:
    # Single release path used by all three panic triggers. ``panic_state`` is a closure
    # over mutable forensic fields so ``main`` stays free of object attributes.
    panic_state: dict[str, str | float] = {"reason": "", "heartbeat_age_s": 0.0}
    panic_event = threading.Event()

    last_heartbeat = time.monotonic()

    def trigger_panic(reason: str) -> None:
        # Capture age at first observation; subsequent triggers from the polling loop
        # would overwrite with a later (and even larger) age so keep the earliest.
        if panic_state["reason"] == "":
            panic_state["reason"] = reason
            panic_state["heartbeat_age_s"] = time.monotonic() - last_heartbeat
        panic_event.set()

    def read_loop() -> None:
        nonlocal last_heartbeat
        while True:
            try:
                # read(1) blocks until a byte arrives or the pipe is closed (EOF).
                b = sys.stdin.buffer.read(1)
                if not b:
                    # EOF: parent gone — release straight away. Best-effort duplicate of the
                    # parent's own atexit release; KEYUP on already-up keys is a no-op.
                    trigger_panic("eof")
                    return
                last_heartbeat = time.monotonic()
            except Exception:
                trigger_panic("read_error")
                return

    reader = threading.Thread(target=read_loop, daemon=True)
    reader.start()

    stall_ticks = 0
    while not panic_event.is_set():
        now = time.monotonic()
        age = now - last_heartbeat
        if age > STALL_AFTER_S:
            stall_ticks += 1
            if stall_ticks >= STALL_TICK_THRESHOLD:
                trigger_panic("stall")
                break
        else:
            # Reset on any fresh observation that age is still healthy — a single late tick
            # inside the threshold must not bank a false-positive streak across a recovery.
            stall_ticks = 0
        time.sleep(POLL_INTERVAL_S)

    if panic_event.is_set():
        panic_release_all()
        # Forensic telemetry: stderr is parent-discarded by default but harmless to write.
        with contextlib.suppress(Exception):
            reason = panic_state["reason"]
            age = float(panic_state["heartbeat_age_s"])
            sys.stderr.write(
                f"[watchdog] panic_reason={reason} heartbeat_age={age:.3f}s\n"
            )
            sys.stderr.flush()


if __name__ == "__main__":
    main()
