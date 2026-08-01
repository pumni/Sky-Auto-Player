"""Run the process-isolated native Raw Input calibration command."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

from sky_music.platform.win32.native_calibration import (
    NativeCalibrationError,
    run_native_calibration,
)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mode", choices=("quick", "full"), default="quick")
    parser.add_argument("--output", type=Path, default=Path(".cache/calibration-native.json"))
    parser.add_argument(
        "--cache",
        type=Path,
        default=Path(".cache/input_latency.json"),
        help="validated legacy margin cache written after native cleanup succeeds",
    )
    args = parser.parse_args()

    try:
        result = run_native_calibration(
            mode=args.mode,
            output_path=args.output,
            cache_path=args.cache,
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
                "artifact": str(args.output),
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
