from pathlib import Path

import pytest

from sky_music.config import AppConfig
from sky_music.domain.session_context import (
    PlaybackSessionContext,
    merge_session_with_overrides,
)


def test_default_session_uses_one_frame_and_60_fps() -> None:
    session = PlaybackSessionContext.default()

    assert session.hold_frames == 1.0
    assert session.fps == 60
    assert session.display_hold_label() == "hold 1.00f"


@pytest.mark.parametrize("value", [True, 1.2, float("nan"), float("inf"), "1.0"])
def test_invalid_hold_selection_is_rejected(value: object) -> None:
    with pytest.raises(ValueError):
        PlaybackSessionContext(hold_frames=value)  # type: ignore[arg-type]


def test_hold_change_preserves_user_selected_fps() -> None:
    session = PlaybackSessionContext(hold_frames=1.0, fps=120)
    changed = session.with_hold_frames(1.25)

    assert changed.hold_frames == 1.25
    assert changed.fps == 120
    assert changed.display_hold_label() == "hold 1.25f"


def test_merge_overrides_updates_hold_tempo_and_fps() -> None:
    base = PlaybackSessionContext.default()
    merged = merge_session_with_overrides(base, hold_frames=1.5, tempo=0.9, fps=144)

    assert merged.hold_frames == 1.5
    assert merged.tempo_scale == 0.9
    assert merged.fps == 144


def test_metadata_cache_key_changes_when_hold_or_fps_changes() -> None:
    song = Path("songs/example.json")
    base = PlaybackSessionContext.default()

    assert base.metadata_cache_key(song) != base.with_hold_frames(1.25).metadata_cache_key(song)
    assert base.metadata_cache_key(song) != base.with_fps(120).metadata_cache_key(song)


def test_effective_policy_uses_one_hold_source_and_margin_metadata() -> None:
    session = PlaybackSessionContext(hold_frames=1.25, fps=60)
    policy = session.resolve_effective_policy(
        AppConfig(), hold_margin_us=800, hold_margin_source="device_cache"
    )

    assert policy.hold_frames == 1.25
    assert policy.hold_us == policy.min_hold_us == 21_634
    assert policy.min_hold_margin_us == 800
    assert policy.min_hold_margin_source == "device_cache"
    assert policy.down_late_grace_us == 500
    assert policy.focus_restore_grace_us == 100_000


def test_cli_exposes_hold_frames_and_rejects_removed_absolute_flags() -> None:
    import main

    parser = main.build_arg_parser()
    args = parser.parse_args(["--hold-frames", "1.25", "--fps", "120"])
    session = PlaybackSessionContext.from_cli_args(args, AppConfig())

    assert session.hold_frames == 1.25
    assert session.fps == 120
    for old_args in (
        ["--timing-profile", "balanced"],
        ["--hold-ms", "30"],
        ["--min-hold-ms", "10"],
    ):
        with pytest.raises(SystemExit):
            parser.parse_args(old_args)
