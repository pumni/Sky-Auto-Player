from __future__ import annotations

from sky_music.ui.playback_notices import PlaybackNoticeLedger
from sky_music.ui.textual_app.playback_app import PlaybackCard, PlaybackSnapshot


def test_schedule_notice_is_persistent_and_dynamic_notices_are_independent() -> None:
    ledger = PlaybackNoticeLedger(("repeat warning",))

    first = ledger.update(input_path_degraded=True, keys_dropped=2, chord_split_events=1)
    second = ledger.update(input_path_degraded=False, keys_dropped=0)

    assert [notice.message for notice in first.persistent_notices] == ["repeat warning"]
    assert [notice.message for notice in second.persistent_notices] == ["repeat warning"]
    assert len(first.runtime_notices) == 1
    assert not second.runtime_notices
    assert len(second.backend_notices) == 1


def test_playback_card_keeps_schedule_warning_across_polls() -> None:
    card = PlaybackCard(theme_name="aurora")
    card._mode = "playing"
    card._notice_ledger = PlaybackNoticeLedger(("same-key repeat warning",))

    card._snapshot = PlaybackSnapshot(
        current=1.0,
        total=5.0,
        song_name="Test",
        input_path_degraded=True,
    )
    first = "\n".join(card._playing_body(80))

    card._snapshot = PlaybackSnapshot(
        current=1.1,
        total=5.0,
        song_name="Test",
        input_path_degraded=False,
    )
    second = "\n".join(card._playing_body(80))

    assert "same-key repeat warning" in first
    assert "same-key repeat warning" in second
    assert "Input dispatch latency" in first
    assert "Input dispatch latency" not in second
