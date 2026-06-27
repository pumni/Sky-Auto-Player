"""Tái hiện: local_precise@144 -> đổi profile/fps -> quay lại local_precise@144.
Đo min_hold hiệu dụng ở từng bước qua ĐÚNG các hàm persist/load thật."""
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
    persist_default_profile,
    profile_dict_for,
)
from sky_music.domain.session_context import PlaybackSessionContext


def eff_min_hold(cfg, profile="local-precise", fps=144) -> tuple[int, dict]:
    sess = PlaybackSessionContext(profile_name=profile, fps=fps)
    pol = sess.resolve_effective_policy(cfg)
    return int(pol.min_hold_us), dict(profile_dict_for(cfg, "local_precise"))


def show(tag, cfg):
    mh, pdict = eff_min_hold(cfg)
    print(f"  {tag:<52} min_hold(lp@144) = {mh:>7}us   override_dict={pdict}")
    return mh


def main():
    tmp = Path(tempfile.mkdtemp())
    config.CONFIG_PATH = tmp / "config.json"
    print(f"temp config: {config.CONFIG_PATH}\n")

    # --- Baseline: tươi, chưa có file config ---
    clear_config_cache()
    cfg = load_config(force_reload=True)
    base = show("FRESH (no config file)", cfg)

    # --- B1: đổi sang balanced@60 rồi quay lại local-precise@144 (luồng picker) ---
    print("\n[B] picker round-trip qua persist_default_* :")
    persist_default_profile(load_config(), "balanced")
    persist_default_fps(load_config(), 60)
    show("after -> balanced@60", load_config())
    persist_default_profile(load_config(), "local-precise")
    persist_default_fps(load_config(), 144)
    b = show("after -> back to local-precise@144", load_config())

    # --- C: chèn 1 lần calibration (ghi timing_profiles) ở giữa ---
    print("\n[C] có chạy lệnh 'calibration' ở giữa (persist_calibration_defaults) :")
    # giả lập calibration khuyến nghị local_precise với fps khác
    for cal_profile, cal_fps in (("balanced", 60), ("local-precise", 60), ("local-precise", 240)):
        clear_config_cache()
        cfg = load_config(force_reload=True)  # reset về tươi mỗi kịch bản
        persist_calibration_defaults(load_config(), profile_name=cal_profile, tempo_scale=1.0, fps=cal_fps)
        show(f"after calibration({cal_profile}@{cal_fps})", load_config())
        persist_default_profile(load_config(), "local-precise")
        persist_default_fps(load_config(), 144)
        after = show("  then user sets back local-precise@144", load_config())
        flag = "  <-- KHÁC baseline!" if after != base else ""
        print(f"      baseline={base}  after={after}{flag}")

    # --- D: round-trip qua DISK (process restart) ---
    print("\n[D] ghi đĩa rồi reload (mô phỏng khởi động lại app) :")
    clear_config_cache()
    cfg = load_config(force_reload=True)
    persist_calibration_defaults(load_config(), profile_name="local-precise", tempo_scale=1.0, fps=240)
    persist_default_profile(load_config(), "local-precise")
    persist_default_fps(load_config(), 144)
    raw = json.loads(config.CONFIG_PATH.read_text(encoding="utf-8"))
    print("  config.json timing_profiles =", json.dumps(raw.get("timing_profiles", {})))
    clear_config_cache()
    cfg2 = load_config(force_reload=True)
    d = show("after disk reload, local-precise@144", cfg2)
    print(f"\n  baseline={base}  B(picker only)={b}  D(disk after calib)={d}")


if __name__ == "__main__":
    main()
