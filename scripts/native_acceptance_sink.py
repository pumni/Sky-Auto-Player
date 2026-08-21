"""Create a project-owned window for explicit native SendInput qualification.

The sink only receives and records ordinary window key events. It never emits
input, enumerates other processes, installs hooks, or reads game state. Start it
on the isolated Windows host, copy the printed HWND into ``SKY_NATIVE_TARGET_HWND``,
and keep this window as the intended input target while running the acceptance
benchmark.
"""

from __future__ import annotations

import argparse
import json
import os
import time
from pathlib import Path
from typing import Any

WINDOW_TITLE = "Sky Auto Player — Native Acceptance Sink"


def _parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--ready-file",
        type=Path,
        help="optional JSON file receiving the sink PID and HWND",
    )
    parser.add_argument(
        "--event-log",
        type=Path,
        help="optional JSONL file receiving key events observed by this sink",
    )
    parser.add_argument(
        "--duration-seconds",
        type=float,
        default=0.0,
        help="close after this many seconds; zero keeps the sink open",
    )
    return parser.parse_args()


def _write_json(path: Path, payload: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")


def main() -> int:
    if os.name != "nt":
        raise SystemExit("native acceptance sink is Windows-only")

    args = _parse_args()
    if args.duration_seconds < 0:
        raise SystemExit("--duration-seconds must be non-negative")

    try:
        import tkinter as tk
    except ImportError as exc:
        raise SystemExit(f"Tkinter is required for the native acceptance sink: {exc}") from exc

    root: Any = tk.Tk()
    root.title(WINDOW_TITLE)
    root.geometry("620x240")
    root.minsize(520, 180)

    status = tk.StringVar(value="Waiting for the explicit benchmark command…")
    tk.Label(root, text=WINDOW_TITLE, font=("Segoe UI", 14, "bold")).pack(pady=(18, 4))
    tk.Label(
        root,
        text=(
            "This project-owned window is the only permitted real-input sink.\n"
            "Keep it as the intended foreground target during SendInput runs."
        ),
        justify="center",
    ).pack(pady=4)
    tk.Label(root, textvariable=status).pack(pady=8)

    root.update_idletasks()
    hwnd = int(root.winfo_id())
    ready_payload = {
        "pid": os.getpid(),
        "hwnd": hwnd,
        "title": WINDOW_TITLE,
        "process": "native_acceptance_sink.py",
        "input_policy": "receives_only; benchmark must use SendInput",
    }
    print(json.dumps(ready_payload), flush=True)
    if args.ready_file is not None:
        _write_json(args.ready_file, ready_payload)

    event_log = args.event_log.open("a", encoding="utf-8") if args.event_log else None
    event_counts = {"key_press": 0, "key_release": 0}

    def record(kind: str, event: Any) -> None:
        event_counts[kind] += 1
        status.set(
            f"Observed {event_counts['key_press']} key presses / "
            f"{event_counts['key_release']} key releases"
        )
        if event_log is not None:
            event_log.write(
                json.dumps(
                    {
                        "kind": kind,
                        "keysym": str(event.keysym),
                        "keycode": int(event.keycode),
                        "observed_ns": time.perf_counter_ns(),
                    }
                )
                + "\n"
            )
            event_log.flush()

    def close() -> None:
        if event_log is not None:
            event_log.close()
        root.destroy()

    root.bind("<KeyPress>", lambda event: record("key_press", event), add="+")
    root.bind("<KeyRelease>", lambda event: record("key_release", event), add="+")
    root.protocol("WM_DELETE_WINDOW", close)
    if args.duration_seconds:
        root.after(round(args.duration_seconds * 1_000), close)
    root.mainloop()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
