from __future__ import annotations

from sky_music.ui.timing_guidance import (
    FPS_MODAL_INFO,
    PROFILE_MODAL_INFO,
    fps_play_advisory,
)


def test_fps_advisory_none_when_fps_60():
    assert fps_play_advisory(fps=60, short_note_count=5) is None


def test_fps_advisory_none_when_zero_short_notes():
    assert fps_play_advisory(fps=144, short_note_count=0) is None


def test_fps_advisory_contains_fps_and_frame_ms():
    advisory = fps_play_advisory(fps=144, short_note_count=3)
    assert advisory is not None
    assert "144" in advisory
    assert "16.7" in advisory
    assert "shorter" in advisory.lower()


def test_fps_modal_info_denies_auto_detect():
    lower = FPS_MODAL_INFO.lower()
    assert "not" in lower
    assert "auto-detect" in lower
    assert "read the game" in lower


def test_profile_modal_info_mentions_keys():
    assert "local-precise" in PROFILE_MODAL_INFO
    assert "audience-safe" in PROFILE_MODAL_INFO
    assert "balanced" in PROFILE_MODAL_INFO


def test_picker_fps_modal_passes_info_text():
    from sky_music.ui.textual_app.modals import OptionModal
    from sky_music.ui.timing_guidance import FPS_MODAL_INFO
    modal = OptionModal("FPS", [], info_text=FPS_MODAL_INFO)
    assert modal.info_text == FPS_MODAL_INFO


def test_picker_profile_modal_passes_info_text():
    from sky_music.ui.textual_app.modals import OptionModal
    from sky_music.ui.timing_guidance import PROFILE_MODAL_INFO
    modal = OptionModal("Timing Profile", [], info_text=PROFILE_MODAL_INFO)
    assert modal.info_text == PROFILE_MODAL_INFO