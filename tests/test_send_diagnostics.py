"""Partial-send diagnostics + musical no-retry policy.

Note-on (key_up=False): never open a second SendInput for the remainder — that split is the
one sender-side place chord atomicity breaks (late/ghost notes). Unsent keys are reported as
dropped; the backend/coordinator mark them DROPPED_BACKEND.

Note-off / release_all: remainder may still be completed (stuck-key safety).
"""

from sky_music.platform.win32 import inputs


def test_full_send_records_no_partial(monkeypatch):
    inputs.reset_send_diagnostics()
    monkeypatch.setattr(inputs.user32, "SendInput", lambda count, array, size: count)
    monkeypatch.setattr(inputs, "is_sky_active", lambda: True)

    landed = inputs.send_scan_code_batch_trusted((0x15, 0x16, 0x17), key_up=False)

    assert landed == 3
    diag = inputs.get_send_diagnostics()
    assert diag == {
        "partial_send_events": 0,
        "chord_split_events": 0,
        "keys_deferred": 0,
        "keys_dropped": 0,
        "keys_retried": 0,
        "zero_progress_retries": 0,
        "send_while_unfocused": 0,
        "impossible_same_key_repeats": 0,
    }


def test_partial_note_on_same_frame_recovery(monkeypatch):
    """Musical path: same-frame retry recovers remainder; exactly two SendInput calls, sleepless."""
    inputs.reset_send_diagnostics()
    calls: list[int] = []

    def fake_send_input(count, array, size):
        calls.append(count)
        if len(calls) == 1:
            return count - 1  # drop one
        return count  # retry recovers it

    def fake_sleep(*args, **kwargs):
        raise RuntimeError("Note-on same-frame retry must not sleep")

    monkeypatch.setattr(inputs.user32, "SendInput", fake_send_input)
    monkeypatch.setattr(inputs, "is_sky_active", lambda: True)
    monkeypatch.setattr(inputs.time, "sleep", fake_sleep)
    monkeypatch.setattr(inputs, "_retry_wait_seconds", fake_sleep)

    landed = inputs.send_scan_code_batch_trusted((0x15, 0x16, 0x17), key_up=False)

    assert landed == 3
    assert calls == [3, 1], "one initial send, one immediate retry"
    diag = inputs.get_send_diagnostics()
    assert diag["partial_send_events"] == 1
    assert diag["chord_split_events"] == 1
    assert diag["keys_deferred"] == 1
    assert diag["keys_retried"] == 1
    assert diag["keys_dropped"] == 0


def test_partial_note_off_completes_remainder(monkeypatch):
    """Safety path: release may finish remaining keys so they cannot stick."""
    inputs.reset_send_diagnostics()
    calls: list[int] = []

    def fake_send_input(count, array, size):
        calls.append(count)
        # First call partial; follow-up (via send_input_batch) succeeds fully.
        if len(calls) == 1:
            return count - 1
        return count

    monkeypatch.setattr(inputs.user32, "SendInput", fake_send_input)
    monkeypatch.setattr(inputs, "is_sky_active", lambda: True)

    landed = inputs.send_scan_code_batch_trusted((0x15, 0x16, 0x17), key_up=True)

    assert landed == 3
    assert len(calls) >= 2, "note-off must complete remainder"
    diag = inputs.get_send_diagnostics()
    assert diag["chord_split_events"] == 1
    assert diag["keys_retried"] == 1
    assert diag["keys_dropped"] == 0


def test_single_key_note_on_zero_progress_retries_once_then_drops(monkeypatch):
    inputs.reset_send_diagnostics()
    calls: list[int] = []

    def fake_send_input(count, array, size):
        calls.append(count)
        return 0  # blocked — musical path does not spin-retry, just immediate retry

    def fake_sleep(*args, **kwargs):
        raise RuntimeError("Note-on retry must not sleep")

    monkeypatch.setattr(inputs.user32, "SendInput", fake_send_input)
    monkeypatch.setattr(inputs, "is_sky_active", lambda: True)
    monkeypatch.setattr(inputs.time, "sleep", fake_sleep)
    monkeypatch.setattr(inputs, "_retry_wait_seconds", fake_sleep)

    landed = inputs.send_scan_code_batch_trusted((0x15,), key_up=False)

    assert landed == 0
    assert calls == [1, 1], "one initial send, exactly one immediate retry"
    diag = inputs.get_send_diagnostics()
    assert diag["chord_split_events"] == 0
    assert diag["partial_send_events"] == 1
    assert diag["keys_dropped"] == 1
    assert diag["keys_retried"] == 0


def test_send_while_unfocused_counted_when_inactive():
    inputs.reset_send_diagnostics()

    # Counter is now explicitly bumped by the orchestration loop when focus is lost
    inputs.note_send_while_unfocused()

    diag = inputs.get_send_diagnostics()
    assert diag["send_while_unfocused"] == 1


def test_backend_partial_note_on_tracks_only_landed_keys(monkeypatch):
    """WinSendInputBackend must not invent active state for unsent chord members.
    Test prefix invariant: send 4, first lands 2, retry lands 1."""
    from sky_music.infrastructure.backend import WinSendInputBackend

    inputs.reset_send_diagnostics()
    calls: list[int] = []

    def fake_send_input(count, array, size):
        calls.append(count)
        if len(calls) == 1:
            return 2  # first call lands 2 of 4
        if len(calls) == 2:
            return 1  # retry lands 1 of 2
        return 0

    monkeypatch.setattr(inputs.user32, "SendInput", fake_send_input)

    backend = WinSendInputBackend()
    result = backend.key_down((0x11, 0x12, 0x13, 0x14))

    assert result.sent == (0x11, 0x12, 0x13)
    assert result.success is False
    assert backend.active_keys == {0x11, 0x12, 0x13}
    assert 0x14 not in backend.active_keys
    assert 0x14 not in backend.possibly_active_keys
