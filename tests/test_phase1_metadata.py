from __future__ import annotations

from sky_music.domain.domain import Millis, Note, NoteKey, Song
from sky_music.domain.scheduler import build_key_actions
from sky_music.domain.scheduler_types import FrameTimingPolicy


def test_schedule_metadata_short_note_counter() -> None:
    # A song with a note shorter than one 60 fps frame (e.g., 5ms hold).
    # 60 FPS frame is ~16.6ms.
    song = Song(
        name="short",
        notes=(
            Note(key=NoteKey("Key1"), time_ms=Millis(0)),
            Note(key=NoteKey("Key1"), time_ms=Millis(5)),
        ),
    )
    
    # 144 fps profile => 1 frame is ~6.9ms.
    policy_144 = FrameTimingPolicy.from_profile_name("local_precise", fps=144)
    # The minimum hold is 6945 us. The same-key gap is 5ms (5000 us).
    # It will trigger compression/drops, but the authored hold will be 6945 us? No, the second note overlaps.
    
    # Wait, the check in scheduler is: "authored down->up hold < ceil(1e6/60)".
    # A single note's authored hold is min_hold_us for that policy.
    # At 144 FPS, min_hold_us is ceil(1_000_000 / 144) = 6945 us.
    # 6945 us < 16667 us, so it is counted as a short note!
    meta_144 = build_key_actions(song, policy=policy_144)
    assert meta_144.sub_60fps_frame_notes > 0
    assert any("short note(s) are shorter than one 60 fps frame" in w for w in meta_144.warnings)

    # 60 fps profile => 1 frame is 16667 us. min_hold_us is 16667 us.
    # 16667 is not < 16667 (it's equal). So it's not a short note.
    policy_60 = FrameTimingPolicy.from_profile_name("local_precise", fps=60)
    meta_60 = build_key_actions(song, policy=policy_60)
    assert meta_60.sub_60fps_frame_notes == 0
    assert not any("short note(s) are shorter than one 60 fps frame" in w for w in meta_60.warnings)


def test_telemetry_summary_schema_min_hold_assumes_fps() -> None:
    from sky_music.infrastructure.backend import DryRunBackend
    from sky_music.infrastructure.timing import SleepPolicy
    from sky_music.orchestration.engine import PLAYBACK_FINISHED, PlaybackEngine
    
    class FakeClock:
        def __init__(self) -> None:
            self.time_us = 0
        def now_us(self) -> int:
            return self.time_us
            
    class FakeSleeper:
        def __init__(self, clock: FakeClock) -> None:
            self.clock = clock
        def sleep(self, seconds: float) -> None:
            self.clock.time_us += max(1, int(seconds * 1_000_000))
            
    class NullControls:
        def poll(self) -> str | None:
            return None
            
    from sky_music.domain.scheduler_types import (
        ActionKind,
        KeyAction,
        Microseconds,
        ScanCode,
    )

    clock = FakeClock()
    engine = PlaybackEngine(
        song=Song(name="test", notes=()),
        actions=(
            KeyAction(
                kind=ActionKind.DOWN,
                scan_codes=(ScanCode(21),),
                at_us=Microseconds(0),
                reason="test",
            ),
            KeyAction(
                kind=ActionKind.UP,
                scan_codes=(ScanCode(21),),
                at_us=Microseconds(1000),
                reason="test",
            ),
        ),
        backend=DryRunBackend(),
        controls=NullControls(),
        telemetry_enabled=True,
        require_focus=False,
        clock=clock,
        sleeper=FakeSleeper(clock),
        sleep_policy=SleepPolicy(spin_threshold_us=-1),
        min_hold_us=10_000,
        fps=144,
        use_dispatch_thread=False,
        dispatch_lead_us=0,
    )
    assert engine.play() == PLAYBACK_FINISHED
    summary = engine.telemetry.get_summary()
    assert summary is not None
    assert summary["runtime_options"]["min_hold_assumes_fps"] == 144

