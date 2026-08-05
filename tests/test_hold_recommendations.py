from sky_music.domain.analyzer import analyze_schedule
from sky_music.domain.domain import Microseconds, ScanCode
from sky_music.domain.scheduler_types import ActionKind, KeyAction, ScheduleMetadata


def _metadata(*, risky: int = 0, compressed: int = 0, polyphony: int = 1) -> ScheduleMetadata:
    actions = (
        KeyAction(
            kind=ActionKind.DOWN,
            scan_codes=(ScanCode(21),),
            at_us=Microseconds(0),
        ),
    )
    return ScheduleMetadata(
        actions=actions,
        source_duration_us=Microseconds(1_000_000),
        playback_duration_us=Microseconds(1_000_000),
        risky_same_key_repeats=risky,
        compressed_holds=compressed,
        max_polyphony=polyphony,
        note_count=1,
    )


def test_any_repeat_stress_wins_over_polyphony() -> None:
    assert analyze_schedule(_metadata(risky=1, polyphony=5), current_hold_frames=1.5).suggested_hold_frames == 1.0
    assert analyze_schedule(_metadata(compressed=1, polyphony=8), current_hold_frames=1.5).suggested_hold_frames == 1.0


def test_polyphony_without_repeat_stress_can_recommend_long_hold() -> None:
    assert analyze_schedule(_metadata(polyphony=5), current_hold_frames=1.0).suggested_hold_frames == 1.5
