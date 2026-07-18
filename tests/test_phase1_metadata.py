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
    
    # 144 fps local_precise: min_hold_us = ceil(1e6/144) + 500us margin = 6945 + 500 = 7445 us.
    # The scheduler counts a note as "short" when its authored down->up hold is below one
    # 60 fps frame (ceil(1e6/60) = 16667 us); 7445 < 16667, so both notes count.
    policy_144 = FrameTimingPolicy.from_profile_name("local_precise", fps=144)
    meta_144 = build_key_actions(song, policy=policy_144)
    assert meta_144.sub_60fps_frame_notes > 0
    assert any("short note(s) are shorter than one 60 fps frame" in w for w in meta_144.warnings)

    # 60 fps local_precise: min_hold_us = 16667 + 500 = 17167 us >= one 60 fps frame,
    # so nothing is counted as short (and the fps<=60 warning gate does not fire anyway).
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


def test_schedule_metadata_gap_below_frame_repeats() -> None:
    # Synthetic song, local_precise@144 (post-Phase-3 min_hold 7 445)
    # two same-key notes 8 ms apart
    song = Song(
        name="gap_below_frame",
        notes=(
            Note(key=NoteKey("Key1"), time_ms=Millis(0)),
            Note(key=NoteKey("Key1"), time_ms=Millis(8)),
        ),
    )
    policy_144 = FrameTimingPolicy.from_profile_name("local_precise", fps=144)
    meta_144 = build_key_actions(song, policy=policy_144)
    
    # interval = 8000 >= 7445 (feasible).
    # hold = min(7445, 8000) = 7445.
    # gap = 8000 - 7445 = 555 us.
    # frame_us = ceil(1e6/144) = 6945 us.
    # Since 555 < 6945, gap is below one frame!
    assert meta_144.gap_below_frame_repeats == 1
    assert any("release-to-repress gap is below one game frame" in w for w in meta_144.warnings)
    assert any(d.code == "gap_below_frame" for d in meta_144.diagnostics)

    # Two notes 500 ms apart -> gap should be 500ms - 7.445ms = 492.555ms >= one frame -> gap_below_frame_repeats == 0
    song_far = Song(
        name="gap_far",
        notes=(
            Note(key=NoteKey("Key1"), time_ms=Millis(0)),
            Note(key=NoteKey("Key1"), time_ms=Millis(500)),
        ),
    )
    meta_far = build_key_actions(song_far, policy=policy_144)
    assert meta_far.gap_below_frame_repeats == 0

    # A severe (interval < min_hold) case: two notes 5 ms apart
    # interval = 5000 < 7445 (severe/impossible).
    # should be counted ONLY as impossible_repeat, gap_below_frame_repeats == 0
    song_severe = Song(
        name="severe",
        notes=(
            Note(key=NoteKey("Key1"), time_ms=Millis(0)),
            Note(key=NoteKey("Key1"), time_ms=Millis(5)),
        ),
    )
    meta_severe = build_key_actions(song_severe, policy=policy_144)
    assert meta_severe.impossible_same_key_repeats == 1
    assert meta_severe.gap_below_frame_repeats == 0


def test_schedule_warnings_include_fps_advisory_message() -> None:
    """Phase C: ScheduleMetadata.warnings contains the FPS advisory with expected message fragment."""
    song = Song(
        name="fps_advisory",
        notes=(
            Note(key=NoteKey("Key1"), time_ms=Millis(0)),
            Note(key=NoteKey("Key1"), time_ms=Millis(5)),
        ),
    )
    policy_144 = FrameTimingPolicy.from_profile_name("local_precise", fps=144)
    meta = build_key_actions(song, policy=policy_144)
    assert any("shorter than one 60 fps frame" in w for w in meta.warnings)
    assert any("Lower fps in the profile" in w for w in meta.warnings)


