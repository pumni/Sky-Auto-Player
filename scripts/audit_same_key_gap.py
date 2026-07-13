"""Audit same-key interval & up-gap across the song corpus, swept over hold.

Conclusion: if min(min_same_key_up_gap_us) >= frame_us at EVERY tested hold level,
then H-SAMEKEY does NOT bind -> exclude from analysis (matches timing-principles §5.2/§6).
"""
from __future__ import annotations

import math
from pathlib import Path

from sky_music.domain.parser import parse_song_file
from sky_music.domain.scheduler import build_key_actions
from sky_music.domain.scheduler_types import FrameTimingPolicy, TimingPolicy
from sky_music.layouts import SKY_15_KEY_PROFILE

FPS = 60
FRAME_US = math.ceil(1_000_000 / FPS)
HOLD_FRAMES_SWEEP = [1.0, 1.25, 1.5, 2.0]


def policy_for(hold_frames: float) -> FrameTimingPolicy:
    # Decouple hold from min_hold: hold increases, min_hold stays fixed at 1 frame.
    base = TimingPolicy.from_dict({
        "min_hold_frames": 1.0,
        "min_hold_unframed_us": 22_000,
        "hold_frames": hold_frames,
    })
    return FrameTimingPolicy.from_timing_policy(base, fps=FPS)


def main() -> None:
    songs = sorted(Path("songs").glob("*.json"))
    if not songs:
        print("No songs found in songs/")
        return
    print(f"FPS={FPS} frame_us={FRAME_US}  (H-SAMEKEY binds khi up_gap < frame_us)")
    for hold_frames in HOLD_FRAMES_SWEEP:
        policy = policy_for(hold_frames)
        worst_gap = None
        worst_song = None
        worst_interval = None
        for sp in songs:
            try:
                song = parse_song_file(sp, SKY_15_KEY_PROFILE)
                res = build_key_actions(song, policy=policy)
            except Exception as e:  # song failed to parse — skip, log note
                print(f"  [skip] {sp.name}: {e}")
                continue
            gap = res.min_same_key_up_gap_us
            if gap is not None and (worst_gap is None or gap < worst_gap):
                worst_gap = gap
                worst_song = sp.name
                worst_interval = res.shortest_same_key_interval_us
        binds = worst_gap is not None and worst_gap < FRAME_US
        print(
            f"hold={hold_frames:>4}f hold_us={int(policy.hold_us):>6} "
            f"min_up_gap={worst_gap} (song={worst_song}, "
            f"shortest_interval={worst_interval})  H-SAMEKEY_binds={binds}"
        )


if __name__ == "__main__":
    main()
