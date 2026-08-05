from sky_music.ui.timing_guidance import (
    FPS_MODAL_INFO,
    HOLD_MODAL_INFO,
    fps_play_advisory,
)


def test_fps_advisory_none_when_fps_60() -> None:
    assert fps_play_advisory(fps=60, short_note_count=5) is None


def test_fps_advisory_explains_matching_sky_fps() -> None:
    advisory = fps_play_advisory(fps=144, short_note_count=3)

    assert advisory is not None
    assert "144" in advisory
    assert "shorter" in advisory.lower()
    assert "match sky" in advisory.lower()


def test_fps_modal_requires_manual_synchronization() -> None:
    lower = FPS_MODAL_INFO.lower()
    assert "same fps" in lower
    assert "not read" in lower
    assert "auto-detect" in lower


def test_hold_modal_describes_exact_supported_choices() -> None:
    assert "1.0 frame" in HOLD_MODAL_INFO
    assert "1.25 frames" in HOLD_MODAL_INFO
    assert "1.5 frames" in HOLD_MODAL_INFO
    assert "game frames" in HOLD_MODAL_INFO
