"""§1 STUTTER MEASUREMENT LIVE — orchestrate WASAPI loopback capture and analysis.

Starts a background recording of the default render endpoint, waits for the user to
finish playing the song, locates the generated playback telemetry CSV, and performs
the after-send measurement analysis.

Install optional dependency via:
    uv add --dev soundcard
"""
from __future__ import annotations

import argparse
import sys
import time
import threading
from datetime import datetime
from pathlib import Path

# Add tests directory to python path for importing sister modules
sys.path.insert(0, str(Path(__file__).parent))

try:
    from audio_loopback import capture_loopback_to_wav
    HAS_CAPTURE = True
except ImportError:
    HAS_CAPTURE = False

from measure_stutter import (
    analyze_and_report,
    detect_onsets,
    read_wav_mono,
    load_sent_downs,
)


def locate_newest_csv() -> Path | None:
    """Return the Path to the newest playback_telemetry_*.csv file in the logs directory."""
    logs_dir = Path("logs")
    if not logs_dir.exists():
        return None
    csv_files = list(logs_dir.glob("playback_telemetry_*.csv"))
    if not csv_files:
        return None
    csv_files.sort(key=lambda p: p.stat().st_mtime, reverse=True)
    return csv_files[0]


def main() -> int:
    try:
        sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    except Exception:
        pass

    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    ap.add_argument("--duration", "-d", type=float, default=None,
                    help="Capture duration in seconds. If omitted, captures until Enter is pressed.")
    ap.add_argument("--csv", type=str, default=None,
                    help="Path to specific telemetry CSV file. If omitted, auto-finds the newest one.")
    ap.add_argument("--fps", type=int, default=0, help="game FPS, to print the frame period for context")
    ap.add_argument("--tol-ms", type=float, default=120.0,
                    help="match tolerance heard<->sent (default 120)")
    ap.add_argument("--gap-ms", type=float, default=30.0,
                    help="report a STUTTER GAP when audio IOI exceeds scheduled IOI by this much (default 30)")
    ap.add_argument("--top", type=int, default=15, help="how many largest gaps to list")
    ap.add_argument("--sensitivity", type=float, default=1.5, help="onset detector: higher = fewer onsets")
    ap.add_argument("--min-gap-ms", type=float, default=40.0, help="onset detector refractory period")
    ap.add_argument("--hop-ms", type=float, default=5.0, help="onset detector envelope resolution")
    args = ap.parse_args()

    if not HAS_CAPTURE:
        print(
            "ERROR: WASAPI loopback capture is unavailable because the 'soundcard' library is missing.\n"
            "FIX: Install the optional dependency with:\n"
            "  uv add --dev soundcard",
            file=sys.stderr
        )
        return 1

    # 1. Setup paths
    logs_dir = Path("logs")
    logs_dir.mkdir(parents=True, exist_ok=True)
    timestamp = datetime.now().strftime("%Y%m%d-%H%M%S")
    out_wav = logs_dir / f"live_capture_{timestamp}.wav"

    # 2. Start background recording
    stop_event = threading.Event()
    thread = threading.Thread(
        target=capture_loopback_to_wav,
        args=(out_wav,),
        kwargs={
            "stop_event": stop_event,
            "max_seconds": args.duration,
        },
        daemon=True
    )

    print(f"Preparing to capture defaults to: {out_wav}")
    try:
        thread.start()
    except RuntimeError as e:
        print(f"\nERROR: Could not start audio capture: {e}\n", file=sys.stderr)
        return 1

    # Check if thread failed immediately (e.g. no audio device)
    time.sleep(0.1)
    if not thread.is_alive() and not out_wav.exists():
        print("\nERROR: Audio capture thread failed to initialize.\n", file=sys.stderr)
        return 1

    # 3. Wait for recording to complete
    try:
        if args.duration is None:
            input("\n>>> Recording started. Play your song now. PRESS ENTER WHEN FINISHED to stop capture...")
        else:
            print(f"\n>>> Recording started. Play your song now. Capturing for {args.duration} seconds...")
            while thread.is_alive():
                thread.join(timeout=0.1)
    except KeyboardInterrupt:
        print("\nInterrupt received. Stopping capture...")
    finally:
        stop_event.set()
        thread.join()

    print(f"Wrote captured WAV to: {out_wav}")

    # 4. Locate CSV
    csv_path_str = args.csv
    if not csv_path_str:
        newest = locate_newest_csv()
        if newest:
            csv_path_str = str(newest)
            print(f"Auto-located newest telemetry CSV: {csv_path_str}")
        else:
            print(
                "\nERROR: No telemetry CSV found in logs/ directory.\n"
                "Please run play with --debug-csv or specify the CSV manually with --csv.\n",
                file=sys.stderr
            )
            return 1
    else:
        print(f"Using specified telemetry CSV: {csv_path_str}")

    # 5. Load inputs
    try:
        print("Reading WAV ...")
        samples, sr = read_wav_mono(str(out_wav))
        print(f"  {len(samples)} samples @ {sr} Hz ({len(samples)/sr:.1f}s). Detecting onsets...")
        heard = detect_onsets(
            samples, sr, hop_ms=args.hop_ms, min_gap_ms=args.min_gap_ms, sensitivity=args.sensitivity
        )
        print(f"[HEARD] {len(heard)} audio onsets detected")
        
        sent, intended, counts = load_sent_downs(csv_path_str)
    except Exception as e:
        print(f"\nERROR: Failed to load analysis inputs: {e}\n", file=sys.stderr)
        return 1

    # 6. Analyze and report
    code = analyze_and_report(
        sent,
        heard,
        fps=args.fps,
        tol_ms=args.tol_ms,
        gap_ms=args.gap_ms,
        top=args.top,
        intended=intended,
        outcome_counts=counts,
    )
    
    # 7. Echo command to re-run
    print(f"\nTo re-analyze this run later, use:")
    print(f"  uv run python tests/measure_stutter.py {out_wav} {csv_path_str} --fps {args.fps}")
    
    return code


if __name__ == "__main__":
    try:
        sys.exit(main())
    except SystemExit as se:
        sys.exit(se.code)
