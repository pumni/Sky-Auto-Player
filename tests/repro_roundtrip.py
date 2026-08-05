"""Reproduce: hold 1.0@144 -> switch hold/fps -> back to hold 1.0@144.
Measure effective min_hold at each step through the REAL persist/load functions."""
from __future__ import annotations

import json
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

import sky_music.config as config
from sky_music.config import (
    clear_config_cache,
    load_config,
    persist_calibration_defaults,
    persist_default_fps,
)
from sky_music.domain.session_context import PlaybackSessionContext


def eff_min_hold(cfg, hold_frames=1.0, fps=144) -> tuple[int, dict]:
    sess = PlaybackSessionContext(hold_frames=hold_frames, fps=fps)
    pol = sess.resolve_effective_policy(cfg)
    return int(pol.min_hold_us), {"hold_frames": hold_frames, "fps": fps}


def show(tag, cfg):
    mh, pdict = eff_min_hold(cfg)
    print(f"  {tag:<52} min_hold(hold@144) = {mh:>7}us   selection={pdict}")
    return mh


def main():
    tmp = Path(tempfile.mkdtemp())
    config.CONFIG_PATH = tmp / "config.json"
    print(f"temp config: {config.CONFIG_PATH}\n")

    # --- Baseline: fresh, no config file yet ---
    clear_config_cache()
    cfg = load_config(force_reload=True)
    base = show("FRESH (no config file)", cfg)

    # --- B1: switch to 1.25@60 then back to 1.0@144 (picker flow) ---
    print("\n[B] picker round-trip via persist_playback_defaults:")
    config.persist_playback_defaults(load_config(), hold_frames=1.25, tempo_scale=1.0, fps=60)
    persist_default_fps(load_config(), 60)
    show("after -> hold 1.25@60", load_config())
    config.persist_playback_defaults(load_config(), hold_frames=1.0, tempo_scale=1.0, fps=144)
    b = show("after -> back to hold 1.0@144", load_config())

    # --- C: insert calibration runs in between ---
    print("\n[C] calibration run in between (persist_calibration_defaults) :")
    # simulate calibration recommending local_precise with a different fps
    for cal_hold, cal_fps in ((1.25, 60), (1.0, 60), (1.0, 240)):
        clear_config_cache()
        cfg = load_config(force_reload=True)  # reset to fresh per scenario
        persist_calibration_defaults(load_config(), hold_frames=cal_hold, tempo_scale=1.0, fps=cal_fps)
        show(f"after calibration(hold {cal_hold}@{cal_fps})", load_config())
        config.persist_playback_defaults(load_config(), hold_frames=1.0, tempo_scale=1.0, fps=144)
        after = show("  then user sets back hold 1.0@144", load_config())
        flag = "  <-- DIFFERENT from baseline!" if after != base else ""
        print(f"      baseline={base}  after={after}{flag}")

    # --- D: round-trip through DISK (simulate app restart) ---
    print("\n[D] write to disk then reload (simulate app restart) :")
    clear_config_cache()
    cfg = load_config(force_reload=True)
    persist_calibration_defaults(load_config(), hold_frames=1.0, tempo_scale=1.0, fps=240)
    config.persist_playback_defaults(load_config(), hold_frames=1.0, tempo_scale=1.0, fps=144)
    raw = json.loads(config.CONFIG_PATH.read_text(encoding="utf-8"))
    print("  config.json hold selection =", json.dumps({"hold_frames": raw.get("default_hold_frames"), "fps": raw.get("game_fps")}))
    clear_config_cache()
    cfg2 = load_config(force_reload=True)
    d = show("after disk reload, local-precise@144", cfg2)
    print(f"\n  baseline={base}  B(picker only)={b}  D(disk after calib)={d}")


if __name__ == "__main__":
    main()
