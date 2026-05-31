import sys
from pathlib import Path
import pytest

src_dir = Path(__file__).parent.parent / "src"
sys.path.insert(0, str(src_dir))

import main

def test_timing_profile_parsing():
    """Verify that specifying a timing profile configures the global TimingPolicy correctly."""
    parser = main.build_arg_parser()
    
    # 1. Test balanced profile (default)
    args = parser.parse_args(["--timing-profile", "balanced"])
    main.configure_from_args(args)
    assert main.TIMING_POLICY.hold_us == 24_000
    assert main.TIMING_POLICY.min_hold_us == 12_000
    assert main.TIMING_POLICY.release_gap_us == 3_000
    
    # 2. Test local-precise profile
    args = parser.parse_args(["--timing-profile", "local-precise"])
    main.configure_from_args(args)
    # 20000us base scales to 20834us at 60 FPS (1.25 frames)
    assert main.TIMING_POLICY.hold_us in (20_000, 20_834)
    assert main.TIMING_POLICY.min_hold_us == 12_000
    
    # 3. Test remote-safe profile
    args = parser.parse_args(["--timing-profile", "remote-safe"])
    main.configure_from_args(args)
    assert main.TIMING_POLICY.hold_us == 30_000
    assert main.TIMING_POLICY.min_hold_us == 15_000

def test_timing_overrides_parsing():
    """Verify custom command-line overrides successfully take precedence over profile policies."""
    parser = main.build_arg_parser()
    args = parser.parse_args([
        "--timing-profile", "balanced",
        "--hold-ms", "30",
        "--min-hold-ms", "15",
        "--release-gap-ms", "5",
        "--repeat-release-gap-ms", "4"
    ])
    main.configure_from_args(args)
    
    assert main.TIMING_POLICY.hold_us == 30_000
    assert main.TIMING_POLICY.min_hold_us == 15_000
    assert main.TIMING_POLICY.release_gap_us == 5_000
    assert main.TIMING_POLICY.repeat_release_gap_us == 4_000

def test_dry_run_simulation_flag():
    """Verify that specifying dry-run and debug-csv configures active states cleanly."""
    parser = main.build_arg_parser()
    args = parser.parse_args(["--dry-run", "--debug-csv"])
    main.configure_from_args(args)
    
    assert main.DRY_RUN_MODE is True
    assert main.TELEMETRY_CSV_ENABLED is True
