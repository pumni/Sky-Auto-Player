from __future__ import annotations

from sky_music.ui.playback_notices import PlaybackNoticeLedger
from sky_music.ui.textual_app.playback_app import PlaybackCard, PlaybackSnapshot


def test_schedule_notice_is_persistent_and_dynamic_notices_are_independent() -> None:
    ledger = PlaybackNoticeLedger(("repeat warning",))

    first = ledger.update(input_path_degraded=True, keys_dropped=2, chord_split_events=1)
    second = ledger.update(input_path_degraded=False, keys_dropped=0)

    assert [notice.message for notice in first.persistent_notices] == ["repeat warning"]
    assert [notice.message for notice in second.persistent_notices] == ["repeat warning"]
    assert not first.runtime_notices
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
    assert "Windows input injection latency is elevated" not in first
    assert "Windows input injection latency is elevated" not in second


def test_latency_signals_are_distinct_from_backend_rejection() -> None:
    ledger = PlaybackNoticeLedger()

    send = ledger.update(sendinput_path_degraded=True)
    assert [notice.code for notice in send.runtime_notices] == ["sendinput-slow"]
    assert not send.backend_notices

    wait = PlaybackNoticeLedger().update(wait_path_degraded=True)
    assert [notice.code for notice in wait.runtime_notices] == ["scheduler-wake-slow"]

    bookkeeping = PlaybackNoticeLedger().update(bookkeeping_degraded=True)
    assert [notice.code for notice in bookkeeping.runtime_notices] == [
        "native-bookkeeping-slow"
    ]

    rejected = PlaybackNoticeLedger().update(
        chords_rejected=2,
        authored_keys_rejected=3,
        sendinput_partial_events=1,
    )
    assert [notice.code for notice in rejected.backend_notices] == [
        "native-input-rejection",
        "partial-input-packet",
    ]
    assert "2 chord(s), 3 authored key(s)" in rejected.backend_notices[0].message


def test_recovered_partial_release_has_neutral_warning() -> None:
    notices = PlaybackNoticeLedger().update(recovered_partial_up_retries=1)
    assert [notice.code for notice in notices.runtime_notices] == [
        "recovered-partial-up-retry"
    ]
    assert "partially accepted a release packet" in notices.runtime_notices[0].message
