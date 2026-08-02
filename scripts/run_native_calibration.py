"""Run the process-isolated native Raw Input calibration command."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

from sky_music.platform.win32.native_calibration import (
    QUICK_CALIBRATION_TIMEOUT_SECONDS,
    NativeCalibrationError,
    run_native_calibration,
)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mode", choices=("diagnostic", "quick", "full"), default="quick")
    parser.add_argument("--kind", choices=("down", "up"))
    parser.add_argument("--class", dest="class_name", choices=("hot", "cold"))
    parser.add_argument("--polyphony", type=int)
    parser.add_argument("--samples", type=int)
    parser.add_argument(
        "--checkpoint-dir",
        type=Path,
        default=Path(".cache/calibration-full"),
        help="per-bucket checkpoint directory for full mode",
    )
    parser.add_argument(
        "--resume",
        action="store_true",
        help="resume only from a checkpoint with exact matching provenance",
    )
    parser.add_argument(
        "--budget-seconds",
        "--timeout-seconds",
        dest="timeout_seconds",
        type=float,
        default=None,
        help=(
            "hard native measurement budget in seconds (1..120); defaults to "
            f"{QUICK_CALIBRATION_TIMEOUT_SECONDS:g}s"
        ),
    )
    parser.add_argument("--output", type=Path, default=Path(".cache/calibration-native.json"))
    parser.add_argument(
        "--cache",
        type=Path,
        default=Path(".cache/input_latency.json"),
        help="validated legacy margin cache written after native cleanup succeeds",
    )
    parser.add_argument(
        "--failure-report",
        type=Path,
        default=None,
        help="failure report path (diagnostic defaults beside --output)",
    )
    args = parser.parse_args()

    try:
        result = run_native_calibration(
            mode=args.mode,
            output_path=args.output,
            cache_path=args.cache,
            timeout_seconds=args.timeout_seconds,
            checkpoint_dir=args.checkpoint_dir,
            resume=args.resume,
            kind=args.kind,
            class_name=args.class_name,
            polyphony=args.polyphony,
            samples=args.samples,
            failure_report_path=args.failure_report,
        )
    except NativeCalibrationError as exc:
        print(f"native calibration failed: {exc}", file=sys.stderr)
        return 1

    print(
        json.dumps(
            {
                "evidence_kind": result["evidence_kind"],
                "version": result["version"],
                "measured_attempted": result["measured_attempted"],
                "measured_anomalous": result["measured_anomalous"],
                "artifact": str(
                    args.output if args.mode != "full" else args.checkpoint_dir
                ),
                "acceptance_eligible": result.get("acceptance_eligible", False),
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
