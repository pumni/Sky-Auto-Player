"""Validate all per-bucket native calibration evidence and publish the cache."""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

from sky_music.platform.win32.native_calibration import (
    NativeCalibrationError,
    finalize_native_calibration,
)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--checkpoint-dir", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--cache", type=Path, required=True)
    args = parser.parse_args()
    try:
        result = finalize_native_calibration(
            checkpoint_dir=args.checkpoint_dir,
            output_path=args.output,
            cache_path=args.cache,
        )
    except NativeCalibrationError as exc:
        print(f"native calibration finalization failed: {exc}", file=sys.stderr)
        return 1
    print(
        f"finalized {result['measured_attempted']} measured samples from "
        f"{len(result['orchestration_configuration']['polyphonies']) * 4} buckets; "
        f"trusted cache={args.cache}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
