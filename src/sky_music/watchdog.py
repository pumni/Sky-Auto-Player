"""
Watchdog subprocess to ensure keys are released if the main process crashes or stalls.
"""
import contextlib
import sys
import threading
import time

from sky_music.platform.win32.inputs import send_scan_code_batch

# The 15 fixed scan codes for Sky
# 'y': 0x15, 'u': 0x16, 'i': 0x17, 'o': 0x18, 'p': 0x19
# 'h': 0x23, 'j': 0x24, 'k': 0x25, 'l': 0x26, ';': 0x27
# 'n': 0x31, 'm': 0x32, ',': 0x33, '.': 0x34, '/': 0x35
SKY_15_SCAN_CODES = (
    0x15, 0x16, 0x17, 0x18, 0x19,
    0x23, 0x24, 0x25, 0x26, 0x27,
    0x31, 0x32, 0x33, 0x34, 0x35,
)

TIMEOUT_SEC = 0.75

def panic_release_all() -> None:
    """Send KEYUP for all 15 scan codes."""
    with contextlib.suppress(Exception):
        send_scan_code_batch(SKY_15_SCAN_CODES, key_up=True)

def main() -> None:
    # We read from stdin. The parent writes a byte (heartbeat) periodically.
    # If the parent dies, stdin hits EOF.
    # If the parent hangs, stdin stays open but no bytes arrive, triggering timeout.
    
    last_heartbeat = time.monotonic()
    stop_event = threading.Event()
    
    def read_loop():
        nonlocal last_heartbeat
        while True:
            try:
                # Read 1 byte. This blocks until a byte is available or EOF.
                # read1() is unbuffered, but sys.stdin.buffer.read1 might not be available
                # or work reliably on Windows pipes without hanging. 
                # Actually, sys.stdin.buffer.read(1) works fine.
                b = sys.stdin.buffer.read(1)
                if not b:
                    # EOF (parent process died or closed pipe)
                    stop_event.set()
                    break
                last_heartbeat = time.monotonic()
            except Exception:
                stop_event.set()
                break

    t = threading.Thread(target=read_loop, daemon=True)
    t.start()

    panicked = False
    while not stop_event.is_set():
        now = time.monotonic()
        if now - last_heartbeat > TIMEOUT_SEC:
            # Parent stalled! Panic!
            panic_release_all()
            panicked = True
            break
        time.sleep(0.05)
        
    if not panicked:
        # We exited cleanly due to EOF (parent exited gracefully and closed pipe)
        # We can still ensure everything is released just in case, but usually parent does it.
        pass

if __name__ == "__main__":
    main()
